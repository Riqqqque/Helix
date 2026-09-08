//! Server-relative, descriptor-anchored file access. No shell or host path input.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use helix_privd::ServerFileRequest;
use rustix::fs::{self as sys, AtFlags, Mode, OFlags, RenameFlags};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BinaryHeap, HashMap},
    fs::{File, Metadata},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    os::{
        fd::{AsRawFd as _, OwnedFd},
        unix::fs::MetadataExt as _,
    },
    path::{Component, Path},
    sync::Mutex,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const MAX_UPLOAD: u64 = 8 * 1024 * 1024 * 1024;
pub const CHUNK: usize = 1024 * 1024;
const TEXT_LIMIT: u64 = 1024 * 1024;
const IDLE: Duration = Duration::from_secs(600);
pub const ACTIONS: &[&str] = &[
    "list",
    "stat",
    "read",
    "write",
    "create",
    "mkdir",
    "move",
    "trash",
    "download",
    "upload_begin",
    "upload_chunk",
    "upload_status",
    "upload_finish",
    "upload_abort",
];

#[derive(Default)]
pub struct ServerFiles {
    uploads: Mutex<HashMap<String, Upload>>,
}

struct Upload {
    server: String,
    path: String,
    size: u64,
    written: u64,
    sha256: String,
    hash: Sha256,
    expected_revision: Option<String>,
    touched: Instant,
    temporary: Temporary,
}

struct Temporary {
    parent: OwnedFd,
    file: File,
}

impl ServerFiles {
    pub fn execute(
        &self,
        server: &str,
        root: &Path,
        uid: u32,
        request: ServerFileRequest,
    ) -> Result<Value, String> {
        let root = open_root(root)?;
        use ServerFileRequest as R;
        match request {
            R::List {
                path,
                cursor,
                limit,
            } => list(&root, &path, cursor.as_deref(), limit),
            R::Stat { path } => {
                let (_, _, file) = existing(&root, &path)?;
                Ok(info(&path, &file.metadata().map_err(error)?))
            }
            R::Read { path } => {
                let (_, _, mut file) = existing(&root, &path)?;
                let before = regular(&file)?;
                if before.len() > TEXT_LIMIT {
                    return Err("text reads are limited to 1 MiB; use chunked download for larger or binary files".into());
                }
                let mut bytes = Vec::new();
                (&mut file)
                    .take(TEXT_LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .map_err(error)?;
                check_revision(&file, &revision(&before))?;
                if bytes.len() as u64 > TEXT_LIMIT {
                    return Err("file grew during reading".into());
                }
                let content = String::from_utf8(bytes)
                    .map_err(|_| "this file is not UTF-8 text; use download".to_owned())?;
                Ok(json!({"path": path, "content": content, "revision": revision(&before)}))
            }
            R::Download {
                path,
                offset,
                length,
                expected_revision,
            } => {
                if length == 0 || length as usize > CHUNK {
                    return Err("download chunks must be between 1 byte and 1 MiB".into());
                }
                let (_, _, mut file) = existing(&root, &path)?;
                let metadata = regular(&file)?;
                check_revision(&file, &expected_revision)?;
                if offset > metadata.len() {
                    return Err("download offset exceeds file size".into());
                }
                file.seek(SeekFrom::Start(offset)).map_err(error)?;
                let mut data = vec![0; (metadata.len() - offset).min(u64::from(length)) as usize];
                file.read_exact(&mut data).map_err(error)?;
                check_revision(&file, &expected_revision)?;
                Ok(
                    json!({"path": path, "offset": offset, "next_offset": offset + data.len() as u64,
                    "size": metadata.len(), "revision": expected_revision, "eof": offset + data.len() as u64 == metadata.len(),
                    "sha256": hex(&Sha256::digest(&data)), "data_base64": STANDARD.encode(data)}),
                )
            }
            R::Mkdir { path } => {
                let (parent, name) = parent(&root, &path)?;
                sys::mkdirat(&parent, &name, Mode::from_raw_mode(0o750)).map_err(error)?;
                let directory = open_dir(&parent, &name)?;
                sys::fchown(
                    &directory,
                    Some(rustix::process::Uid::from_raw(uid)),
                    Some(rustix::process::Gid::from_raw(uid)),
                )
                .map_err(error)?;
                sys::fsync(&parent).map_err(error)?;
                Ok(json!({"path": path}))
            }
            R::Create { path } => {
                let temp = temporary(&root, &path)?;
                publish(&root, &path, &temp, uid, None)
            }
            R::Write {
                path,
                content,
                expected_revision,
            } => {
                if content.len() as u64 > TEXT_LIMIT {
                    return Err(
                        "text writes are limited to 1 MiB; use chunked upload for larger files"
                            .into(),
                    );
                }
                let mut temp = temporary(&root, &path)?;
                temp.file.write_all(content.as_bytes()).map_err(error)?;
                publish(&root, &path, &temp, uid, Some(&expected_revision))
            }
            R::Move {
                path,
                destination,
                expected_revision,
            } => {
                let (source_parent, source_name, file) = existing(&root, &path)?;
                check_revision(&file, &expected_revision)?;
                let (destination_parent, destination_name) = parent(&root, &destination)?;
                sys::renameat_with(
                    &source_parent,
                    &source_name,
                    &destination_parent,
                    &destination_name,
                    RenameFlags::NOREPLACE,
                )
                .map_err(error)?;
                sys::fsync(&source_parent).map_err(error)?;
                sys::fsync(&destination_parent).map_err(error)?;
                Ok(json!({"path": destination}))
            }
            R::Trash {
                path,
                expected_revision,
            } => {
                if path == ".helix-trash" || path.starts_with(".helix-trash/") {
                    return Err(
                        "trash is already recoverable; move an item out to restore it".into(),
                    );
                }
                let (parent, name, file) = existing(&root, &path)?;
                check_revision(&file, &expected_revision)?;
                let trash = trash_directory(&root)?;
                let recovery = recovery_name(&name);
                sys::renameat_with(&parent, &name, &trash, &recovery, RenameFlags::NOREPLACE)
                    .map_err(error)?;
                sys::fsync(&parent).map_err(error)?;
                sys::fsync(&trash).map_err(error)?;
                Ok(json!({"path": path, "recovery_path": format!(".helix-trash/{recovery}")}))
            }
            R::UploadBegin {
                path,
                size,
                sha256,
                expected_revision,
            } => {
                if size > MAX_UPLOAD
                    || sha256.len() != 64
                    || !sha256.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(
                        "upload needs a SHA-256 digest and a size from zero to 8 GiB".into(),
                    );
                }
                let mut uploads = self.uploads.lock().map_err(error)?;
                uploads.retain(|_, u| u.touched.elapsed() < IDLE);
                if uploads.len() >= 2 {
                    return Err("two uploads are already active; finish or abort one first".into());
                }
                if uploads
                    .values()
                    .any(|u| u.server == server && u.path == path)
                {
                    return Err("an upload to this server path is already active".into());
                }
                check_destination(&root, &path, expected_revision.as_deref())?;
                let temporary = temporary(&root, &path)?;
                let id = Uuid::new_v4().to_string();
                uploads.insert(
                    id.clone(),
                    Upload {
                        server: server.into(),
                        path,
                        size,
                        written: 0,
                        sha256: sha256.to_ascii_lowercase(),
                        hash: Sha256::new(),
                        expected_revision,
                        touched: Instant::now(),
                        temporary,
                    },
                );
                Ok(
                    json!({"upload_id": id, "bytes_written": 0, "size": size, "max_chunk_bytes": CHUNK, "idle_timeout_seconds": 600}),
                )
            }
            other => self.upload_request(server, &root, uid, other),
        }
    }

    fn upload_request(
        &self,
        server: &str,
        root: &OwnedFd,
        uid: u32,
        request: ServerFileRequest,
    ) -> Result<Value, String> {
        use ServerFileRequest as R;
        let id = match &request {
            R::UploadChunk { upload_id, .. }
            | R::UploadStatus { upload_id }
            | R::UploadFinish { upload_id }
            | R::UploadAbort { upload_id } => upload_id.clone(),
            _ => return Err("unsupported file operation".into()),
        };
        let mut uploads = self.uploads.lock().map_err(error)?;
        uploads.retain(|_, u| u.touched.elapsed() < IDLE);
        let upload = uploads
            .get_mut(&id)
            .filter(|u| u.server == server)
            .ok_or_else(|| {
                "upload is not active for this server; it may have expired or the broker restarted"
                    .to_owned()
            })?;
        upload.touched = Instant::now();
        match request {
            R::UploadStatus { .. } => Ok(
                json!({"upload_id": id, "path": upload.path, "bytes_written": upload.written, "size": upload.size}),
            ),
            R::UploadChunk {
                offset,
                data_base64,
                ..
            } => {
                if data_base64.len() > CHUNK.div_ceil(3) * 4 {
                    return Err("upload chunk exceeds 1 MiB".into());
                }
                let bytes = STANDARD
                    .decode(data_base64)
                    .map_err(|_| "invalid base64 chunk".to_owned())?;
                if bytes.is_empty()
                    || bytes.len() > CHUNK
                    || offset != upload.written
                    || offset.saturating_add(bytes.len() as u64) > upload.size
                {
                    return Err("chunk offset or size does not match this upload; read upload_status before resuming".into());
                }
                // A partial write error is reconciled by seeking and rewriting this same offset.
                upload
                    .temporary
                    .file
                    .seek(SeekFrom::Start(offset))
                    .map_err(error)?;
                upload.temporary.file.write_all(&bytes).map_err(error)?;
                upload
                    .temporary
                    .file
                    .set_len(offset + bytes.len() as u64)
                    .map_err(error)?;
                upload.hash.update(&bytes);
                upload.written += bytes.len() as u64;
                Ok(json!({"upload_id": id, "bytes_written": upload.written, "size": upload.size}))
            }
            R::UploadFinish { .. } => {
                if upload.written != upload.size
                    || upload.temporary.file.metadata().map_err(error)?.len() != upload.size
                    || hex(&upload.hash.clone().finalize()) != upload.sha256
                {
                    return Err(
                        "upload size or SHA-256 does not match; the destination was not changed"
                            .into(),
                    );
                }
                let result = publish(
                    root,
                    &upload.path,
                    &upload.temporary,
                    uid,
                    upload.expected_revision.as_deref(),
                )?;
                uploads.remove(&id);
                Ok(result)
            }
            R::UploadAbort { .. } => {
                uploads.remove(&id);
                Ok(json!({"aborted": true}))
            }
            _ => Err("unsupported upload operation".into()),
        }
    }
}

fn error(_: impl std::fmt::Display) -> String {
    "server file operation failed; check the path, permissions, available space and whether the destination already exists".into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn recovery_name(name: &str) -> String {
    format!(
        "{}-{}",
        Uuid::new_v4(),
        name.chars().take(48).collect::<String>()
    )
}

fn parts(path: &str) -> Result<Vec<&str>, String> {
    if path.len() > 4096
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return Err("use a relative server path without control characters or traversal".into());
    }
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.iter().any(|p| {
        p.is_empty() || matches!(*p, "." | "..") || p.len() > 255 || p.starts_with(".helix-api-")
    }) {
        return Err("invalid or reserved server path component".into());
    }
    Ok(parts)
}

fn flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}
fn open_dir(fd: &OwnedFd, name: &str) -> Result<OwnedFd, String> {
    sys::openat(fd, name, flags(), Mode::empty()).map_err(error)
}
fn open_root(path: &Path) -> Result<OwnedFd, String> {
    if !path.is_absolute() {
        return Err("server root is not absolute".into());
    }
    let mut fd = sys::open("/", flags(), Mode::empty()).map_err(error)?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                fd = open_dir(
                    &fd,
                    name.to_str()
                        .ok_or_else(|| "invalid server root".to_owned())?,
                )?
            }
            _ => return Err("invalid server root".into()),
        }
    }
    Ok(fd)
}
fn directory(root: &OwnedFd, path: &str) -> Result<OwnedFd, String> {
    let mut fd = open_dir(root, ".")?;
    for part in parts(path)? {
        fd = open_dir(&fd, part)?;
    }
    Ok(fd)
}
fn parent(root: &OwnedFd, path: &str) -> Result<(OwnedFd, String), String> {
    let mut components = parts(path)?;
    let name = components
        .pop()
        .ok_or_else(|| "the server root cannot be changed".to_owned())?;
    if path == ".helix-trash" {
        return Err("the recovery directory cannot be changed".into());
    }
    Ok((directory(root, &components.join("/"))?, name.into()))
}
fn existing(root: &OwnedFd, path: &str) -> Result<(OwnedFd, String, File), String> {
    let (parent, name) = parent(root, path)?;
    let fd = sys::openat(
        &parent,
        &name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(error)?;
    let file = File::from(fd);
    let m = file.metadata().map_err(error)?;
    if !(m.is_file() || m.is_dir()) || (m.is_file() && m.nlink() != 1) {
        return Err("only regular files and directories are accessible; links and special files are refused".into());
    }
    Ok((parent, name, file))
}
fn regular(file: &File) -> Result<Metadata, String> {
    let m = file.metadata().map_err(error)?;
    if !m.is_file() || m.nlink() != 1 {
        return Err("a regular, non-linked file is required".into());
    }
    Ok(m)
}
fn revision(m: &Metadata) -> String {
    format!(
        "{:x}-{:x}-{:x}-{:x}-{:x}-{:x}-{:x}",
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec()
    )
}
fn check_revision(file: &File, expected: &str) -> Result<(), String> {
    if expected != revision(&file.metadata().map_err(error)?) {
        return Err("file revision conflict; reread the file and reconcile before changing or downloading it".into());
    }
    Ok(())
}
fn info(path: &str, m: &Metadata) -> Value {
    json!({"path": path, "kind": if m.is_dir() { "directory" } else { "file" }, "size": m.len(), "revision": revision(m)})
}

fn list(root: &OwnedFd, path: &str, cursor: Option<&str>, limit: u16) -> Result<Value, String> {
    if !(1..=200).contains(&limit) || cursor.is_some_and(|c| c.len() > 255) {
        return Err("list limit must be 1 to 200 and cursor must be a returned filename".into());
    }
    let fd = directory(root, path)?;
    let mut buffer = Vec::<u8>::with_capacity(8192);
    let mut reader = sys::RawDir::new(&fd, buffer.spare_capacity_mut());
    let mut candidates = BinaryHeap::new();
    let mut omitted = 0;
    while let Some(entry) = reader.next() {
        let entry = entry.map_err(error)?;
        let Ok(name) = entry.file_name().to_str() else {
            omitted += 1;
            continue;
        };
        if matches!(name, "." | "..") || name.starts_with(".helix-api-") {
            continue;
        }
        if parts(name).is_err() {
            omitted += 1;
            continue;
        }
        if cursor.is_some_and(|c| name <= c) {
            continue;
        }
        candidates.push(name.to_owned());
        if candidates.len() > usize::from(limit) + 1 {
            candidates.pop();
        }
    }
    let mut names = candidates.into_sorted_vec();
    let more = names.len() > usize::from(limit);
    if more {
        names.pop();
    }
    let next = more.then(|| names.last().cloned()).flatten();
    let mut entries = Vec::new();
    for name in names {
        let relative = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        let metadata = sys::statat(&fd, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(error)?;
        let kind = sys::FileType::from_raw_mode(metadata.st_mode);
        entries.push(json!({"name": name, "path": relative, "size": metadata.st_size,
            "kind": match kind { sys::FileType::Directory => "directory", sys::FileType::RegularFile => "file", sys::FileType::Symlink => "symlink", _ => "other" },
            "restricted": !matches!(kind, sys::FileType::Directory | sys::FileType::RegularFile) || (kind == sys::FileType::RegularFile && metadata.st_nlink != 1)}));
    }
    Ok(json!({"path": path, "entries": entries, "next_cursor": next, "omitted_entries": omitted}))
}
fn temporary(root: &OwnedFd, path: &str) -> Result<Temporary, String> {
    let (parent, _) = parent(root, path)?;
    let file = File::from(
        sys::openat(
            &parent,
            ".",
            OFlags::RDWR | OFlags::TMPFILE | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| "cannot stage this file; check free space and permissions. Server API uploads require a filesystem supporting Linux O_TMPFILE (such as ext4 or XFS)".to_owned())?,
    );
    Ok(Temporary { parent, file })
}
fn trash_directory(root: &OwnedFd) -> Result<OwnedFd, String> {
    match sys::mkdirat(root, ".helix-trash", Mode::from_raw_mode(0o700)) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(e) => return Err(error(e)),
    }
    open_dir(root, ".helix-trash")
}
fn check_destination(root: &OwnedFd, path: &str, expected: Option<&str>) -> Result<(), String> {
    if let Some(expected) = expected {
        let (_, _, file) = existing(root, path)?;
        regular(&file)?;
        check_revision(&file, expected)
    } else {
        let (parent, name) = parent(root, path)?;
        match sys::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => Ok(()),
            _ => Err(
                "destination exists; read its revision and explicitly request replacement".into(),
            ),
        }
    }
}
fn publish(
    root: &OwnedFd,
    path: &str,
    temp: &Temporary,
    uid: u32,
    expected: Option<&str>,
) -> Result<Value, String> {
    check_destination(root, path, expected)?;
    let (destination_parent, name) = parent(root, path)?;
    let a = sys::fstat(&destination_parent).map_err(error)?;
    let b = sys::fstat(&temp.parent).map_err(error)?;
    if a.st_dev != b.st_dev || a.st_ino != b.st_ino {
        return Err("destination directory moved during upload; abort and start again".into());
    }
    let metadata = expected
        .map(|_| existing(root, path).and_then(|(_, _, file)| regular(&file)))
        .transpose()?;
    sys::fchown(
        &temp.file,
        Some(rustix::process::Uid::from_raw(
            metadata.as_ref().map_or(uid, Metadata::uid),
        )),
        Some(rustix::process::Gid::from_raw(
            metadata.as_ref().map_or(uid, Metadata::gid),
        )),
    )
    .map_err(error)?;
    sys::fchmod(
        &temp.file,
        Mode::from_raw_mode(metadata.as_ref().map_or(0o640, |m| m.mode() & 0o777)),
    )
    .map_err(error)?;
    temp.file.sync_all().map_err(error)?;
    let source = format!("/proc/self/fd/{}", temp.file.as_raw_fd());
    let recovery = if expected.is_some() {
        let trash = trash_directory(root)?;
        let recovery = recovery_name(&name);
        sys::linkat(
            sys::CWD,
            &source,
            &trash,
            &recovery,
            AtFlags::SYMLINK_FOLLOW,
        )
        .map_err(error)?;
        sys::fsync(&trash).map_err(error)?;
        if let Err(e) = sys::renameat_with(
            &trash,
            &recovery,
            &destination_parent,
            &name,
            RenameFlags::EXCHANGE,
        ) {
            let _ = sys::unlinkat(&trash, &recovery, AtFlags::empty());
            return Err(error(e));
        }
        sys::fsync(&trash).map_err(error)?;
        Some(format!(".helix-trash/{recovery}"))
    } else {
        sys::linkat(
            sys::CWD,
            &source,
            &destination_parent,
            &name,
            AtFlags::SYMLINK_FOLLOW,
        )
        .map_err(error)?;
        None
    };
    sys::fsync(&destination_parent).map_err(error)?;
    sys::fsync(root).map_err(error)?;
    Ok(
        json!({"path": path, "revision": revision(&temp.file.metadata().map_err(error)?), "recovery_path": recovery}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_privd::ServerFileRequest as R;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    struct Fixture {
        root: tempfile::TempDir,
        files: ServerFiles,
    }
    impl Fixture {
        fn new() -> Self {
            Self {
                // Docker's overlay layer may lack O_TMPFILE; Linux tmpfs supports it.
                root: tempfile::tempdir_in("/dev/shm").unwrap(),
                files: ServerFiles::default(),
            }
        }
        fn call(&self, request: R) -> Result<Value, String> {
            self.files.execute(
                "selected",
                self.root.path(),
                rustix::process::getuid().as_raw(),
                request,
            )
        }
        fn revision(&self, path: &str) -> String {
            self.call(R::Stat { path: path.into() }).unwrap()["revision"]
                .as_str()
                .unwrap()
                .into()
        }
        fn begin(&self, path: &str, bytes: &[u8], expected_revision: Option<String>) -> String {
            self.call(R::UploadBegin {
                path: path.into(),
                size: bytes.len() as u64,
                sha256: hex(&Sha256::digest(bytes)),
                expected_revision,
            })
            .unwrap()["upload_id"]
                .as_str()
                .unwrap()
                .into()
        }
    }

    #[test]
    fn relative_paths_reject_traversal_and_reserved_names() {
        for path in [
            "/etc/passwd",
            "../x",
            "x/../a",
            "a//b",
            "a\\b",
            "a\0b",
            ".",
            "a/",
            ".helix-api-secret",
        ] {
            assert!(parts(path).is_err(), "{path:?}");
        }
        assert!(parts("mods/My mod [1.2]+forge.jar").is_ok());
        assert!(parts("").unwrap().is_empty());
    }

    #[test]
    fn files_cannot_follow_symlinks_hardlinks_or_special_files() {
        let f = Fixture::new();
        let outside = tempfile::tempdir_in("/dev/shm").unwrap();
        fs::write(outside.path().join("secret"), "private").unwrap();
        symlink(outside.path(), f.root.path().join("escape")).unwrap();
        symlink(outside.path().join("secret"), f.root.path().join("link")).unwrap();
        fs::hard_link(outside.path().join("secret"), f.root.path().join("hard")).unwrap();
        sys::mkfifoat(
            sys::CWD,
            f.root.path().join("pipe"),
            Mode::from_raw_mode(0o600),
        )
        .unwrap();
        for path in ["escape/secret", "link", "hard", "pipe"] {
            assert!(f.call(R::Read { path: path.into() }).is_err());
        }
        assert!(
            f.call(R::Create {
                path: "escape/new".into()
            })
            .is_err()
        );
        assert!(!outside.path().join("new").exists());
    }

    #[test]
    fn write_requires_revision_and_keeps_old_contents_recoverable() {
        let f = Fixture::new();
        fs::write(f.root.path().join("config"), "old").unwrap();
        fs::set_permissions(
            f.root.path().join("config"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let revision = f.revision("config");
        assert!(
            f.call(R::Write {
                path: "config".into(),
                content: "wrong".into(),
                expected_revision: "stale".into()
            })
            .is_err()
        );
        let result = f
            .call(R::Write {
                path: "config".into(),
                content: "new".into(),
                expected_revision: revision.clone(),
            })
            .unwrap();
        assert_eq!(
            fs::read_to_string(f.root.path().join("config")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(
                f.root
                    .path()
                    .join(result["recovery_path"].as_str().unwrap())
            )
            .unwrap(),
            "old"
        );
        assert_eq!(
            fs::metadata(f.root.path().join("config")).unwrap().mode() & 0o777,
            0o600
        );
        assert!(
            f.call(R::Write {
                path: "config".into(),
                content: "stale".into(),
                expected_revision: revision
            })
            .is_err()
        );
    }

    #[test]
    fn rename_trash_and_restore_do_not_overwrite_other_files() {
        let f = Fixture::new();
        f.call(R::Mkdir {
            path: "mods".into(),
        })
        .unwrap();
        f.call(R::Create {
            path: "mods/a".into(),
        })
        .unwrap();
        f.call(R::Create {
            path: "mods/b".into(),
        })
        .unwrap();
        assert!(
            f.call(R::Move {
                path: "mods/a".into(),
                destination: "mods/b".into(),
                expected_revision: f.revision("mods/a")
            })
            .is_err()
        );
        let result = f
            .call(R::Trash {
                path: "mods/a".into(),
                expected_revision: f.revision("mods/a"),
            })
            .unwrap();
        let recovery = result["recovery_path"].as_str().unwrap();
        assert!(!f.root.path().join("mods/a").exists());
        f.call(R::Move {
            path: recovery.into(),
            destination: "mods/a".into(),
            expected_revision: f.revision(recovery),
        })
        .unwrap();
        assert!(f.root.path().join("mods/a").exists());
        assert!(
            f.call(R::Trash {
                path: "".into(),
                expected_revision: "".into()
            })
            .is_err()
        );
    }

    #[test]
    fn upload_is_scoped_bounded_and_not_visible_until_verified() {
        let f = Fixture::new();
        let id = f.begin("plugin.jar", b"payload", None);
        assert!(fs::read_dir(f.root.path()).unwrap().next().is_none());
        assert!(
            f.files
                .execute(
                    "another",
                    f.root.path(),
                    0,
                    R::UploadStatus {
                        upload_id: id.clone()
                    }
                )
                .is_err()
        );
        assert!(
            f.call(R::UploadFinish {
                upload_id: id.clone()
            })
            .is_err()
        );
        assert!(
            f.call(R::UploadChunk {
                upload_id: id.clone(),
                offset: 1,
                data_base64: STANDARD.encode(b"payload")
            })
            .is_err()
        );
        f.call(R::UploadChunk {
            upload_id: id.clone(),
            offset: 0,
            data_base64: STANDARD.encode(b"payload"),
        })
        .unwrap();
        assert_eq!(
            f.call(R::UploadStatus {
                upload_id: id.clone()
            })
            .unwrap()["bytes_written"],
            7
        );
        assert!(
            f.call(R::UploadChunk {
                upload_id: id.clone(),
                offset: 0,
                data_base64: STANDARD.encode(b"payload")
            })
            .is_err()
        );
        f.call(R::UploadFinish {
            upload_id: id.clone(),
        })
        .unwrap();
        assert_eq!(
            fs::read(f.root.path().join("plugin.jar")).unwrap(),
            b"payload"
        );
        assert!(f.call(R::UploadFinish { upload_id: id }).is_err());
    }

    #[test]
    fn failed_checksum_abort_and_broker_drop_leave_destination_unchanged() {
        let f = Fixture::new();
        fs::write(f.root.path().join("existing"), b"old").unwrap();
        let id = f.begin("existing", b"good", Some(f.revision("existing")));
        f.call(R::UploadChunk {
            upload_id: id.clone(),
            offset: 0,
            data_base64: STANDARD.encode(b"evil"),
        })
        .unwrap();
        assert!(
            f.call(R::UploadFinish {
                upload_id: id.clone()
            })
            .is_err()
        );
        assert_eq!(fs::read(f.root.path().join("existing")).unwrap(), b"old");
        f.call(R::UploadAbort { upload_id: id }).unwrap();
        f.begin("unfinished", b"bytes", None);
        drop(f.files);
        let names: Vec<_> = fs::read_dir(f.root.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec!["existing"]);
    }

    #[test]
    fn download_has_offsets_revision_and_chunk_checksums() {
        let f = Fixture::new();
        fs::write(f.root.path().join("data"), b"abcde").unwrap();
        let revision = f.revision("data");
        let chunk = f
            .call(R::Download {
                path: "data".into(),
                offset: 2,
                length: 2,
                expected_revision: revision.clone(),
            })
            .unwrap();
        assert_eq!(chunk["data_base64"], STANDARD.encode(b"cd"));
        assert_eq!(chunk["sha256"], hex(&Sha256::digest(b"cd")));
        assert_eq!(chunk["next_offset"], 4);
        assert_eq!(chunk["eof"], false);
        fs::write(f.root.path().join("data"), b"changed").unwrap();
        assert!(
            f.call(R::Download {
                path: "data".into(),
                offset: 4,
                length: 2,
                expected_revision: revision
            })
            .is_err()
        );
    }

    #[test]
    fn list_paginates_and_reports_unsupported_entries() {
        let f = Fixture::new();
        for name in ["a", "b", "c"] {
            fs::write(f.root.path().join(name), "").unwrap();
        }
        let first = f
            .call(R::List {
                path: "".into(),
                cursor: None,
                limit: 2,
            })
            .unwrap();
        assert_eq!(first["entries"].as_array().unwrap().len(), 2);
        assert_eq!(first["next_cursor"], "b");
        let last = f
            .call(R::List {
                path: "".into(),
                cursor: Some("b".into()),
                limit: 2,
            })
            .unwrap();
        assert_eq!(last["entries"][0]["name"], "c");
        assert!(last["next_cursor"].is_null());
    }

    #[test]
    fn zero_length_upload_and_destination_conflicts_are_safe() {
        let f = Fixture::new();
        let id = f.begin("empty", b"", None);
        f.call(R::UploadFinish { upload_id: id }).unwrap();
        let id = f.begin("later", b"", None);
        fs::write(f.root.path().join("later"), b"keep").unwrap();
        assert!(f.call(R::UploadFinish { upload_id: id }).is_err());
        assert_eq!(fs::read(f.root.path().join("later")).unwrap(), b"keep");
    }
}
