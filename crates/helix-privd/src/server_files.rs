//! Server-relative, descriptor-anchored file access. No shell or host path input.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use helix_privd::{MAX_DIRECT_SERVER_FILE_UPLOAD_BYTES, ServerFileRequest};
use rustix::fs::{self as sys, AtFlags, Mode, OFlags, RenameFlags};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BinaryHeap, HashMap},
    fs::{File, Metadata},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    os::{
        fd::{AsRawFd as _, OwnedFd},
        unix::fs::{MetadataExt as _, PermissionsExt as _},
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
    "upload_file",
    "upload_begin",
    "upload_chunk",
    "upload_status",
    "upload_finish",
    "upload_abort",
    "pending_list",
    "pending_discard",
];

/// Changes made while a server runs wait in a store outside the server folder
/// (`<state_root>/pending/<id>`), owned by the broker and out of the game's reach.
const PENDING_INDEX: &str = "pending.json";
const MAX_PENDING: usize = 500;

#[derive(Default)]
pub struct ServerFiles {
    uploads: Mutex<HashMap<String, Upload>>,
    /// Serializes edits to each server's pending index; file writes share the
    /// server lock, so several can stage at once.
    pending: Mutex<()>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingKind {
    Write,
    Delete,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingChange {
    pub path: String,
    pub kind: PendingKind,
    /// The staged file's name inside the pending folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The live file's revision when the change was staged; None for a new file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<String>,
    pub staged_at_unix_ms: u64,
}

impl PendingChange {
    /// What a client passes as `expected_revision` to change a staged file again.
    fn token(&self) -> String {
        format!("pending:{}", self.blob.as_deref().unwrap_or("delete"))
    }
}

/// One change installed by `apply_pending`, with what is needed to undo it.
#[derive(Clone, Debug, Serialize)]
pub struct AppliedChange {
    pub path: String,
    pub kind: PendingKind,
    /// Folders created for this file, removed again if it is reverted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub created_folders: Vec<String>,
    /// Where the replaced or deleted file was kept, relative to the server.
    pub recovery_path: Option<String>,
    /// The live file changed after this change was staged (for example, a plugin
    /// rewrote its config on shutdown). The staged version still won; the other
    /// one is in the Files trash.
    pub changed_since_staged: bool,
}

pub struct AppliedPending {
    pub applied: Vec<AppliedChange>,
    pub failed: Vec<Value>,
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
    /// Begun on a running server: the bytes are written in the pending store and
    /// the finished file is staged.
    staged: bool,
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
        pending: &Path,
        uid: u32,
        request: ServerFileRequest,
    ) -> Result<Value, String> {
        match request {
            ServerFileRequest::PendingList {} => {
                let store = open_store(pending, false)?;
                return self.pending_list_value(store.as_ref());
            }
            ServerFileRequest::PendingDiscard { path } => {
                let store = open_store(pending, false)?;
                return self.discard_pending(store.as_ref(), path.as_deref());
            }
            _ => {}
        }
        let root = open_root(root)?;
        let touched = match &request {
            ServerFileRequest::Write { path, .. }
            | ServerFileRequest::Create { path }
            | ServerFileRequest::Trash { path, .. }
            | ServerFileRequest::UploadFile { path, .. }
            | ServerFileRequest::Move { path, .. } => Some(path.clone()),
            _ => None,
        };
        let finishing = matches!(request, ServerFileRequest::UploadFinish { .. });
        let result = self.execute_open(server, &root, uid, request, None);
        // A finish that staged the upload (begun while the server ran) must not
        // forget the change it just recorded.
        if let Ok(value) = &result
            && value.get("pending") != Some(&Value::Bool(true))
        {
            let finished = finishing
                .then(|| value.get("path").and_then(Value::as_str).map(str::to_owned))
                .flatten();
            if let Some(path) = touched.or(finished) {
                // A direct change to a stopped server wins over a staged one.
                self.forget_pending(pending, &path);
            }
        }
        result
    }

    fn execute_open(
        &self,
        server: &str,
        root: &OwnedFd,
        uid: u32,
        request: ServerFileRequest,
        staging: Option<&OwnedFd>,
    ) -> Result<Value, String> {
        let root = open_dir(root, ".")?;
        use ServerFileRequest as R;
        match request {
            R::PendingList {} | R::PendingDiscard { .. } => {
                Err("unsupported file operation".into())
            }
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
            R::UploadFile {
                path,
                data_base64,
                sha256,
                expected_revision,
            } => {
                if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("upload_file needs a SHA-256 digest".into());
                }
                if data_base64.len() > MAX_DIRECT_SERVER_FILE_UPLOAD_BYTES.div_ceil(3) * 4 {
                    return Err(
                        "upload_file is limited to 3 MiB; use chunked upload for larger files"
                            .into(),
                    );
                }
                let bytes = STANDARD
                    .decode(data_base64)
                    .map_err(|_| "invalid base64 file".to_owned())?;
                if bytes.len() > MAX_DIRECT_SERVER_FILE_UPLOAD_BYTES {
                    return Err(
                        "upload_file is limited to 3 MiB; use chunked upload for larger files"
                            .into(),
                    );
                }
                let sha256 = sha256.to_ascii_lowercase();
                if hex(&Sha256::digest(&bytes)) != sha256 {
                    return Err(
                        "upload_file SHA-256 does not match; the destination was not changed"
                            .into(),
                    );
                }
                if let Err(conflict) = check_destination(&root, &path, expected_revision.as_deref())
                {
                    if let Some(existing) =
                        matching_destination(&root, &path, bytes.len() as u64, &sha256)?
                    {
                        return Ok(with_upload_state(existing, true));
                    }
                    return Err(conflict);
                }
                let mut temp = temporary(&root, &path)?;
                temp.file.write_all(&bytes).map_err(error)?;
                Ok(with_upload_state(
                    publish(&root, &path, &temp, uid, expected_revision.as_deref())?,
                    false,
                ))
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
            } => self.begin_upload(server, &root, path, size, sha256, expected_revision, None),
            other => self.upload_request(server, &root, uid, other, staging),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_upload(
        &self,
        server: &str,
        root: &OwnedFd,
        path: String,
        size: u64,
        sha256: String,
        expected_revision: Option<String>,
        staging: Option<&OwnedFd>,
    ) -> Result<Value, String> {
        if size > MAX_UPLOAD || sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("upload needs a SHA-256 digest and a size from zero to 8 GiB".into());
        }
        let sha256 = sha256.to_ascii_lowercase();
        let staged = staging.is_some();
        let mut uploads = self.uploads.lock().map_err(error)?;
        uploads.retain(|_, u| u.touched.elapsed() < IDLE);
        if let Some((id, upload)) = uploads.iter_mut().find(|(_, upload)| {
            upload.server == server
                && upload.path == path
                && upload.size == size
                && upload.sha256 == sha256
                && upload.expected_revision == expected_revision
                && upload.staged == staged
        }) {
            upload.touched = Instant::now();
            return Ok(json!({
                "upload_id": id,
                "bytes_written": upload.written,
                "size": size,
                "max_chunk_bytes": CHUNK,
                "idle_timeout_seconds": 600,
                "resumed": true,
                "completed": false
            }));
        }
        uploads.retain(|_, upload| !(upload.server == server && upload.path == path));
        // A staged upload's destination was already checked against the live
        // server and anything staged; it is checked again when installed.
        if !staged
            && let Err(conflict) = check_destination(root, &path, expected_revision.as_deref())
        {
            if let Some(existing) = matching_destination(root, &path, size, &sha256)? {
                return Ok(with_upload_state(existing, true));
            }
            return Err(conflict);
        }
        if uploads.len() >= 2 {
            return Err("two uploads are already active; finish or abort one first".into());
        }
        let temporary = match staging {
            Some(store) => temporary_in(store)?,
            None => temporary(root, &path)?,
        };
        let id = Uuid::new_v4().to_string();
        uploads.insert(
            id.clone(),
            Upload {
                server: server.into(),
                path,
                size,
                written: 0,
                sha256,
                hash: Sha256::new(),
                expected_revision,
                touched: Instant::now(),
                temporary,
                staged,
            },
        );
        Ok(json!({
            "upload_id": id,
            "bytes_written": 0,
            "size": size,
            "max_chunk_bytes": CHUNK,
            "idle_timeout_seconds": 600,
            "resumed": false,
            "completed": false
        }))
    }

    /// Runs a file request for a server that is running. Reads and uploads in
    /// progress work as usual; changes wait in the pending store and are installed
    /// on the next start, restart, or deploy. The live server is not touched.
    pub fn stage(
        &self,
        server: &str,
        root: &Path,
        pending: &Path,
        uid: u32,
        request: ServerFileRequest,
    ) -> Result<Value, String> {
        use ServerFileRequest as R;
        if matches!(request, R::PendingList {} | R::PendingDiscard { .. }) {
            return self.execute(server, root, pending, uid, request);
        }
        let root = open_root(root)?;
        let store = open_store(pending, true)?
            .ok_or_else(|| "the pending store could not be created".to_owned())?;
        match request {
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
                let bytes = content.into_bytes();
                self.stage_bytes(&root, &store, &path, &bytes, Some(&expected_revision))
            }
            R::Create { path } => self.stage_bytes(&root, &store, &path, &[], None),
            R::UploadFile {
                path,
                data_base64,
                sha256,
                expected_revision,
            } => {
                if data_base64.len() > MAX_DIRECT_SERVER_FILE_UPLOAD_BYTES.div_ceil(3) * 4 {
                    return Err(
                        "upload_file is limited to 3 MiB; use chunked upload for larger files"
                            .into(),
                    );
                }
                let bytes = STANDARD
                    .decode(data_base64)
                    .map_err(|_| "invalid base64 file".to_owned())?;
                if hex(&Sha256::digest(&bytes)) != sha256.to_ascii_lowercase() {
                    return Err(
                        "upload_file SHA-256 does not match; the destination was not changed"
                            .into(),
                    );
                }
                self.stage_bytes(&root, &store, &path, &bytes, expected_revision.as_deref())
            }
            R::Trash {
                path,
                expected_revision,
            } => self.stage_delete(&root, &store, &path, &expected_revision),
            R::Move { .. } => Err("moving files needs the server stopped; stop it first, or upload the file to its new path and delete the old one, which are staged until the next restart".into()),
            R::UploadBegin {
                path,
                size,
                sha256,
                expected_revision,
            } => {
                let expected = {
                    let _index_guard = self.pending.lock().map_err(error)?;
                    let index = read_pending(&store)?;
                    resolve_expected(&root, &index, &path, expected_revision.as_deref())?
                };
                if expected.live.is_some()
                    && let Some(existing) =
                        matching_destination(&root, &path, size, &sha256.to_ascii_lowercase())?
                {
                    let _index_guard = self.pending.lock().map_err(error)?;
                    remove_pending_unlocked(&store, &path)?;
                    return Ok(with_upload_state(existing, true));
                }
                self.begin_upload(server, &root, path, size, sha256, expected.live, Some(&store))
            }
            request @ (R::UploadChunk { .. }
            | R::UploadStatus { .. }
            | R::UploadFinish { .. }
            | R::UploadAbort { .. }) => self.execute_open(server, &root, uid, request, Some(&store)),
            other => self.execute_open(server, &root, uid, other, None),
        }
    }

    fn upload_request(
        &self,
        server: &str,
        root: &OwnedFd,
        uid: u32,
        request: ServerFileRequest,
        staging: Option<&OwnedFd>,
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
                json!({"upload_id": id, "path": upload.path, "bytes_written": upload.written, "size": upload.size, "sha256": upload.sha256}),
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
                if bytes.is_empty() || bytes.len() > CHUNK {
                    return Err("chunk offset or size does not match this upload; read upload_status before resuming".into());
                }
                let end = offset.saturating_add(bytes.len() as u64);
                if offset < upload.written && end <= upload.written {
                    upload
                        .temporary
                        .file
                        .seek(SeekFrom::Start(offset))
                        .map_err(error)?;
                    let mut staged = vec![0; bytes.len()];
                    upload
                        .temporary
                        .file
                        .read_exact(&mut staged)
                        .map_err(error)?;
                    if staged != bytes {
                        return Err("replayed upload chunk does not match staged data".into());
                    }
                    return Ok(
                        json!({"upload_id": id, "bytes_written": upload.written, "size": upload.size, "replayed": true}),
                    );
                }
                if offset != upload.written || end > upload.size {
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
                Ok(
                    json!({"upload_id": id, "bytes_written": upload.written, "size": upload.size, "replayed": false}),
                )
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
                // Uploads begun on a running server are staged; one begun while
                // stopped but finished after a start is staged too.
                let store = if upload.staged {
                    Some(open_dir(&upload.temporary.parent, ".")?)
                } else {
                    staging.map(|store| open_dir(store, ".")).transpose()?
                };
                if let Some(store) = store {
                    let blob = Uuid::new_v4().to_string();
                    upload.temporary.file.sync_all().map_err(error)?;
                    if upload.staged {
                        let source = format!("/proc/self/fd/{}", upload.temporary.file.as_raw_fd());
                        sys::linkat(sys::CWD, &source, &store, &blob, AtFlags::SYMLINK_FOLLOW)
                            .map_err(error)?;
                    } else {
                        upload
                            .temporary
                            .file
                            .seek(SeekFrom::Start(0))
                            .map_err(error)?;
                        let copied = create_blob(&store, &blob).and_then(|mut file| {
                            std::io::copy(&mut upload.temporary.file, &mut file)
                                .and_then(|_| file.sync_all())
                                .map_err(error)
                        });
                        if let Err(cause) = copied {
                            let _ = sys::unlinkat(&store, &blob, AtFlags::empty());
                            return Err(cause);
                        }
                    }
                    let change = PendingChange {
                        path: upload.path.clone(),
                        kind: PendingKind::Write,
                        blob: Some(blob.clone()),
                        size: upload.size,
                        sha256: Some(upload.sha256.clone()),
                        base_revision: upload.expected_revision.clone(),
                        staged_at_unix_ms: unix_ms(),
                    };
                    let result = self.record_pending(&store, change);
                    if result.is_err() {
                        let _ = sys::unlinkat(&store, &blob, AtFlags::empty());
                    }
                    uploads.remove(&id);
                    return result;
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

    fn stage_bytes(
        &self,
        root: &OwnedFd,
        store: &OwnedFd,
        path: &str,
        bytes: &[u8],
        expected_revision: Option<&str>,
    ) -> Result<Value, String> {
        let _index_guard = self.pending.lock().map_err(error)?;
        let index = read_pending(store)?;
        let expected = resolve_expected(root, &index, path, expected_revision)?;
        let sha256 = hex(&Sha256::digest(bytes));
        if expected.live.is_some()
            && let Some(existing) = matching_destination(root, path, bytes.len() as u64, &sha256)?
        {
            // The server already has exactly this file; nothing needs to wait.
            remove_pending_unlocked(store, path)?;
            return Ok(with_upload_state(existing, true));
        }
        let blob = Uuid::new_v4().to_string();
        let written = create_blob(store, &blob).and_then(|mut file| {
            file.write_all(bytes)
                .and_then(|()| file.sync_all())
                .map_err(error)
        });
        if let Err(cause) = written {
            let _ = sys::unlinkat(store, &blob, AtFlags::empty());
            return Err(cause);
        }
        let change = PendingChange {
            path: path.to_owned(),
            kind: PendingKind::Write,
            blob: Some(blob.clone()),
            size: bytes.len() as u64,
            sha256: Some(sha256),
            base_revision: expected.live,
            staged_at_unix_ms: unix_ms(),
        };
        let result = record_pending_unlocked(store, change);
        if result.is_err() {
            let _ = sys::unlinkat(store, &blob, AtFlags::empty());
        }
        result
    }

    fn stage_delete(
        &self,
        root: &OwnedFd,
        store: &OwnedFd,
        path: &str,
        expected_revision: &str,
    ) -> Result<Value, String> {
        if path == ".helix-trash" || path.starts_with(".helix-trash/") {
            return Err("trash is already recoverable; move an item out to restore it".into());
        }
        let _index_guard = self.pending.lock().map_err(error)?;
        let index = read_pending(store)?;
        let expected = resolve_expected(root, &index, path, Some(expected_revision))?;
        let Some(live) = expected.live else {
            // Only a staged new file was there; dropping it is the whole delete.
            remove_pending_unlocked(store, path)?;
            return Ok(json!({"path": path, "pending": false, "discarded_staged_file": true}));
        };
        record_pending_unlocked(
            store,
            PendingChange {
                path: path.to_owned(),
                kind: PendingKind::Delete,
                blob: None,
                size: 0,
                sha256: None,
                base_revision: Some(live),
                staged_at_unix_ms: unix_ms(),
            },
        )
    }

    fn record_pending(&self, store: &OwnedFd, change: PendingChange) -> Result<Value, String> {
        let _index_guard = self.pending.lock().map_err(error)?;
        record_pending_unlocked(store, change)
    }

    /// A direct change to a stopped server wins over anything staged for that path.
    fn forget_pending(&self, pending: &Path, path: &str) {
        let Ok(Some(store)) = open_store(pending, false) else {
            return;
        };
        if let Ok(_index_guard) = self.pending.lock() {
            let _ = remove_pending_unlocked(&store, path);
        }
    }

    fn pending_list_value(&self, store: Option<&OwnedFd>) -> Result<Value, String> {
        let index = match store {
            Some(store) => read_pending(store)?,
            None => Vec::new(),
        };
        let bytes: u64 = index.iter().map(|change| change.size).sum();
        Ok(json!({
            "changes": index.iter().map(|change| json!({
                "path": change.path,
                "kind": change.kind,
                "size": change.size,
                "sha256": change.sha256,
                "revision": change.token(),
                "replaces_existing": change.base_revision.is_some(),
                "staged_at_unix_ms": change.staged_at_unix_ms
            })).collect::<Vec<_>>(),
            "count": index.len(),
            "total_bytes": bytes
        }))
    }

    fn discard_pending(
        &self,
        store: Option<&OwnedFd>,
        path: Option<&str>,
    ) -> Result<Value, String> {
        let _index_guard = self.pending.lock().map_err(error)?;
        let index = match store {
            Some(store) => read_pending(store)?,
            None => Vec::new(),
        };
        let selected: Vec<_> = index
            .iter()
            .filter(|change| path.is_none_or(|path| change.path == path))
            .cloned()
            .collect();
        if let Some(path) = path
            && selected.is_empty()
        {
            return Err(format!("no change is staged for {path}"));
        }
        let Some(store) = store.filter(|_| !selected.is_empty()) else {
            return Ok(json!({"discarded": [], "count": 0}));
        };
        let remaining: Vec<_> = index
            .into_iter()
            .filter(|change| path.is_some_and(|path| change.path != path))
            .collect();
        write_pending(store, &remaining)?;
        for change in &selected {
            remove_blob(store, change.blob.as_deref());
        }
        Ok(json!({
            "discarded": selected.iter().map(|change| change.path.clone()).collect::<Vec<_>>(),
            "count": selected.len()
        }))
    }

    /// Lists what is waiting, for the deploy job and the server page.
    pub fn pending_changes(&self, pending: &Path) -> Result<Vec<PendingChange>, String> {
        match open_store(pending, false)? {
            Some(store) => read_pending(&store),
            None => Ok(Vec::new()),
        }
    }

    /// Installs every staged change into a stopped server. Replaced and deleted
    /// files go to the Files trash so `revert_applied` can put them back. The
    /// staged list is cleared only when everything installed, so after a failure
    /// (or a crash part way) every change is still staged; the caller reverts.
    pub fn apply_pending(
        &self,
        root: &Path,
        pending: &Path,
        uid: u32,
    ) -> Result<AppliedPending, String> {
        let mut applied = Vec::new();
        let mut failed = Vec::new();
        let Some(store) = open_store(pending, false)? else {
            return Ok(AppliedPending { applied, failed });
        };
        let root = open_root(root)?;
        let _index_guard = self.pending.lock().map_err(error)?;
        let index = read_pending(&store)?;
        for change in &index {
            match apply_change(&root, &store, change, uid) {
                Ok(done) => applied.push(done),
                Err(message) => failed.push(json!({"path": change.path, "error": message})),
            }
        }
        if failed.is_empty() && !index.is_empty() {
            if let Err(cause) = write_pending(&store, &[]) {
                // Everything is installed but the list could not be cleared; report
                // it as failed so the caller puts the old files back and the
                // changes stay staged instead of being installed twice.
                failed.push(json!({"path": "(staged list)", "error": cause}));
                return Ok(AppliedPending { applied, failed });
            }
            for change in &index {
                remove_blob(&store, change.blob.as_deref());
            }
        }
        Ok(AppliedPending { applied, failed })
    }

    /// Puts back what `apply_pending` replaced, newest first. Installed versions
    /// stay in the Files trash so nothing is lost.
    pub fn revert_applied(&self, root: &Path, applied: &[AppliedChange]) -> Vec<String> {
        let Ok(root) = open_root(root) else {
            return vec!["the server folder could not be opened".to_owned()];
        };
        let mut errors = Vec::new();
        for change in applied.iter().rev() {
            if let Err(message) = revert_change(&root, change) {
                errors.push(format!("{}: {message}", change.path));
            }
        }
        errors
    }
}

/// Which live revision a staged change is based on. A client may name either the
/// live file's revision or the token of a change already staged for the path.
struct Expected {
    live: Option<String>,
}

fn resolve_expected(
    root: &OwnedFd,
    index: &[PendingChange],
    path: &str,
    expected: Option<&str>,
) -> Result<Expected, String> {
    if let Some(token) = expected
        && token.starts_with("pending:")
    {
        let staged = index
            .iter()
            .find(|change| change.path == path && change.token() == token)
            .ok_or_else(|| "file revision conflict; the staged change was replaced or installed. List pending changes or reread the file".to_owned())?;
        return Ok(Expected {
            live: staged.base_revision.clone(),
        });
    }
    if let Some(expected) = expected {
        check_destination(root, path, Some(expected))?;
        return Ok(Expected {
            live: Some(expected.to_owned()),
        });
    }
    if let Some(staged) = index
        .iter()
        .find(|change| change.path == path && change.kind == PendingKind::Write)
    {
        return Err(format!(
            "a change is already staged for {path}; pass its revision {} to replace it",
            staged.token()
        ));
    }
    live_absent(root, path)?;
    Ok(Expected { live: None })
}

/// The path does not exist on the live server. Missing parent folders are fine:
/// they are created when the change is installed.
fn live_absent(root: &OwnedFd, path: &str) -> Result<(), String> {
    let mut components = parts(path)?;
    let name = components
        .pop()
        .ok_or_else(|| "the server root cannot be changed".to_owned())?;
    if path == ".helix-trash" {
        return Err("the recovery directory cannot be changed".into());
    }
    let mut fd = open_dir(root, ".")?;
    for part in components {
        match sys::openat(&fd, part, flags(), Mode::empty()) {
            Ok(next) => fd = next,
            Err(rustix::io::Errno::NOENT) => return Ok(()),
            Err(cause) => return Err(error(cause)),
        }
    }
    match sys::statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        _ => Err("destination exists; read its revision and explicitly request replacement".into()),
    }
}

/// Opens (or creates) the broker-owned pending store, refusing one that another
/// user owns or can read.
fn open_store(pending: &Path, create: bool) -> Result<Option<OwnedFd>, String> {
    let (Some(parent), Some(name)) = (
        pending.parent(),
        pending.file_name().and_then(|name| name.to_str()),
    ) else {
        return Err("the pending store path is invalid".into());
    };
    if create {
        std::fs::create_dir_all(parent).map_err(error)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(error)?;
    }
    let base = match open_root(parent) {
        Ok(fd) => fd,
        Err(_) if !create => return Ok(None),
        Err(cause) => return Err(cause),
    };
    check_private(&base)?;
    if create {
        match sys::mkdirat(&base, name, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(cause) => return Err(error(cause)),
        }
    }
    let store = match sys::openat(&base, name, flags(), Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) if !create => return Ok(None),
        Err(cause) => return Err(error(cause)),
    };
    check_private(&store)?;
    Ok(Some(store))
}

fn check_private(fd: &OwnedFd) -> Result<(), String> {
    let stat = sys::fstat(fd).map_err(error)?;
    if stat.st_uid != rustix::process::geteuid().as_raw() || stat.st_mode & 0o077 != 0 {
        return Err(
            "the pending store is not private to Helix; staged changes were not used".into(),
        );
    }
    Ok(())
}

fn valid_blob(name: &str) -> bool {
    name.len() == 36 && Uuid::parse_str(name).is_ok()
}

fn create_blob(store: &OwnedFd, blob: &str) -> Result<File, String> {
    sys::openat(
        store,
        blob,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map(File::from)
    .map_err(error)
}

fn remove_blob(store: &OwnedFd, blob: Option<&str>) {
    if let Some(blob) = blob.filter(|blob| valid_blob(blob)) {
        let _ = sys::unlinkat(store, blob, AtFlags::empty());
    }
}

/// An unnamed file in the pending store for a staged chunked upload.
fn temporary_in(store: &OwnedFd) -> Result<Temporary, String> {
    let file = File::from(
        sys::openat(
            store,
            ".",
            OFlags::RDWR | OFlags::TMPFILE | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|_| {
            "cannot stage this upload; check free space for Helix's state folder".to_owned()
        })?,
    );
    Ok(Temporary {
        parent: open_dir(store, ".")?,
        file,
    })
}

fn read_pending(store: &OwnedFd) -> Result<Vec<PendingChange>, String> {
    let file = match sys::openat(
        store,
        PENDING_INDEX,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(Vec::new()),
        Err(cause) => return Err(error(cause)),
    };
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    let index: Vec<PendingChange> = serde_json::from_slice(&bytes).map_err(|_| {
        "the list of staged changes is damaged; discard pending changes and stage them again"
            .to_owned()
    })?;
    if index.iter().any(|change| {
        parts(&change.path).is_err()
            || change.blob.as_deref().is_some_and(|blob| !valid_blob(blob))
            || (change.kind == PendingKind::Write) != change.blob.is_some()
    }) {
        return Err(
            "the list of staged changes is damaged; discard pending changes and stage them again"
                .into(),
        );
    }
    Ok(index)
}

fn record_pending_unlocked(store: &OwnedFd, change: PendingChange) -> Result<Value, String> {
    let mut index = read_pending(store)?;
    let mut replaced = None;
    index.retain(|existing| {
        if existing.path == change.path {
            replaced = existing.blob.clone();
            false
        } else {
            true
        }
    });
    if index.len() >= MAX_PENDING {
        return Err(format!(
            "{MAX_PENDING} changes are already waiting for this server; restart it or run a deploy to install them first"
        ));
    }
    let response = json!({
        "path": change.path,
        "pending": true,
        "kind": change.kind,
        "size": change.size,
        "sha256": change.sha256,
        "revision": change.token(),
        "completed": true,
        "unchanged": false,
        "applies": "on the next start, restart, or deploy"
    });
    index.push(change);
    write_pending(store, &index)?;
    remove_blob(store, replaced.as_deref());
    Ok(response)
}

fn remove_pending_unlocked(store: &OwnedFd, path: &str) -> Result<(), String> {
    let mut index = read_pending(store)?;
    let before = index.len();
    let mut blobs = Vec::new();
    let nested = format!("{path}/");
    index.retain(|change| {
        if change.path == path || change.path.starts_with(&nested) {
            blobs.extend(change.blob.clone());
            false
        } else {
            true
        }
    });
    if index.len() == before {
        return Ok(());
    }
    write_pending(store, &index)?;
    for blob in blobs {
        remove_blob(store, Some(&blob));
    }
    Ok(())
}

fn write_pending(store: &OwnedFd, index: &[PendingChange]) -> Result<(), String> {
    let content = serde_json::to_vec_pretty(index)
        .map_err(|_| "could not record the staged changes".to_owned())?;
    let partial = format!("{PENDING_INDEX}.partial");
    let mut file = File::from(
        sys::openat(
            store,
            &partial,
            OFlags::WRONLY | OFlags::CREATE | OFlags::TRUNC | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(error)?,
    );
    file.write_all(&content).map_err(error)?;
    file.sync_all().map_err(error)?;
    sys::renameat(store, &partial, store, PENDING_INDEX).map_err(error)?;
    sys::fsync(store).map_err(error)
}

/// Opens or creates each folder of `path`'s parent, owned by the server user, and
/// returns the folders it created.
fn ensure_parents(root: &OwnedFd, path: &str, uid: u32) -> Result<Vec<String>, String> {
    let mut components = parts(path)?;
    components.pop();
    let mut fd = open_dir(root, ".")?;
    let mut created = Vec::new();
    let mut so_far = String::new();
    for part in components {
        if !so_far.is_empty() {
            so_far.push('/');
        }
        so_far.push_str(part);
        match sys::mkdirat(&fd, part, Mode::from_raw_mode(0o750)) {
            Ok(()) => {
                let folder = open_dir(&fd, part)?;
                sys::fchown(
                    &folder,
                    Some(rustix::process::Uid::from_raw(uid)),
                    Some(rustix::process::Gid::from_raw(uid)),
                )
                .map_err(error)?;
                sys::fsync(&fd).map_err(error)?;
                created.push(so_far.clone());
                fd = folder;
            }
            Err(rustix::io::Errno::EXIST) => fd = open_dir(&fd, part)?,
            Err(cause) => return Err(error(cause)),
        }
    }
    Ok(created)
}

fn remove_created_folders(root: &OwnedFd, folders: &[String]) {
    for folder in folders.iter().rev() {
        if let Ok((parent, name)) = parent(root, folder) {
            // Only empty folders go; anything the server put there stays.
            let _ = sys::unlinkat(&parent, name.as_str(), AtFlags::REMOVEDIR);
        }
    }
}

fn live_revision(root: &OwnedFd, path: &str) -> Result<Option<String>, String> {
    let (parent, name) = parent(root, path)?;
    match sys::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(cause) => return Err(error(cause)),
        Ok(_) => {}
    }
    let (_, _, file) = existing(root, path)?;
    Ok(Some(revision(&regular(&file)?)))
}

fn apply_change(
    root: &OwnedFd,
    store: &OwnedFd,
    change: &PendingChange,
    uid: u32,
) -> Result<AppliedChange, String> {
    match change.kind {
        PendingKind::Write => {
            let blob = change
                .blob
                .as_deref()
                .filter(|blob| valid_blob(blob))
                .ok_or_else(|| "the staged file is missing".to_owned())?;
            let mut file = File::from(
                sys::openat(
                    store,
                    blob,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|_| "the staged file is missing; stage it again".to_owned())?,
            );
            if regular(&file)?.len() != change.size {
                return Err("the staged file changed size; stage it again".into());
            }
            if let Some(expected) = &change.sha256 {
                let mut hash = Sha256::new();
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let count = file.read(&mut buffer).map_err(error)?;
                    if count == 0 {
                        break;
                    }
                    hash.update(&buffer[..count]);
                }
                if &hex(&hash.finalize()) != expected {
                    return Err(
                        "the staged file does not match its checksum; stage it again".into(),
                    );
                }
                file.seek(SeekFrom::Start(0)).map_err(error)?;
            }
            let created_folders = ensure_parents(root, &change.path, uid)?;
            let installed = (|| {
                let mut temp = temporary(root, &change.path)?;
                std::io::copy(&mut file, &mut temp.file).map_err(error)?;
                let current = live_revision(root, &change.path)?;
                let result = publish(root, &change.path, &temp, uid, current.as_deref())?;
                Ok::<_, String>((result, current))
            })();
            let (result, current) = match installed {
                Ok(done) => done,
                Err(cause) => {
                    remove_created_folders(root, &created_folders);
                    return Err(cause);
                }
            };
            Ok(AppliedChange {
                path: change.path.clone(),
                kind: PendingKind::Write,
                created_folders,
                recovery_path: result["recovery_path"].as_str().map(str::to_owned),
                changed_since_staged: current != change.base_revision,
            })
        }
        PendingKind::Delete => {
            let current = live_revision(root, &change.path)?;
            if current.is_none() {
                return Ok(AppliedChange {
                    path: change.path.clone(),
                    kind: PendingKind::Delete,
                    created_folders: Vec::new(),
                    recovery_path: None,
                    changed_since_staged: change.base_revision.is_some(),
                });
            }
            let (parent, name) = parent(root, &change.path)?;
            let trash = trash_directory(root)?;
            let recovery = recovery_name(&name);
            sys::renameat_with(&parent, &name, &trash, &recovery, RenameFlags::NOREPLACE)
                .map_err(error)?;
            sys::fsync(&parent).map_err(error)?;
            sys::fsync(&trash).map_err(error)?;
            Ok(AppliedChange {
                path: change.path.clone(),
                kind: PendingKind::Delete,
                created_folders: Vec::new(),
                recovery_path: Some(format!(".helix-trash/{recovery}")),
                changed_since_staged: current != change.base_revision,
            })
        }
    }
}

fn revert_change(root: &OwnedFd, change: &AppliedChange) -> Result<(), String> {
    let (parent, name) = parent(root, &change.path)?;
    let trash = trash_directory(root)?;
    let recovery = change
        .recovery_path
        .as_deref()
        .and_then(|path| path.strip_prefix(".helix-trash/"));
    match (&change.kind, recovery) {
        // Swap the old file back; the deployed one takes its place in the trash.
        (PendingKind::Write, Some(recovery)) => {
            sys::renameat_with(&trash, recovery, &parent, &name, RenameFlags::EXCHANGE)
                .map_err(error)?;
        }
        // A new file is moved to the trash.
        (PendingKind::Write, None) => {
            sys::renameat_with(
                &parent,
                &name,
                &trash,
                recovery_name(&name),
                RenameFlags::NOREPLACE,
            )
            .map_err(error)?;
        }
        (PendingKind::Delete, Some(recovery)) => {
            sys::renameat_with(&trash, recovery, &parent, &name, RenameFlags::NOREPLACE)
                .map_err(error)?;
        }
        (PendingKind::Delete, None) => return Ok(()),
    }
    sys::fsync(&parent).map_err(error)?;
    sys::fsync(&trash).map_err(error)?;
    drop(parent);
    remove_created_folders(root, &change.created_folders);
    Ok(())
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Turns an OS error into a sentence about the path, without exposing host paths.
fn error(cause: impl std::fmt::Display) -> String {
    let text = cause.to_string();
    let code = text
        .rsplit_once("os error ")
        .and_then(|(_, rest)| rest.trim_end_matches(')').parse::<i32>().ok());
    match code {
        Some(2) => "that path or one of its folders does not exist; create the folder first".into(),
        Some(17) => "something already exists at that path".into(),
        Some(28 | 122) => "the disk is full; free some space and try again".into(),
        Some(1 | 13) => "permission denied for that path".into(),
        Some(20) => "part of that path is a file, not a folder".into(),
        Some(21) => "that path is a folder, not a file".into(),
        Some(39) => "that folder is not empty".into(),
        Some(40) => "that path is a link; Helix does not follow links".into(),
        Some(30) => "the server storage is read-only".into(),
        Some(36) => "that name is too long".into(),
        _ => "server file operation failed; check the path, permissions, available space and whether the destination already exists".into(),
    }
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

fn matching_destination(
    root: &OwnedFd,
    path: &str,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<Option<Value>, String> {
    let (parent, name) = parent(root, path)?;
    let metadata = match sys::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => metadata,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(cause) => return Err(error(cause)),
    };
    if sys::FileType::from_raw_mode(metadata.st_mode) != sys::FileType::RegularFile
        || metadata.st_nlink != 1
        || u64::try_from(metadata.st_size).ok() != Some(expected_size)
    {
        return Ok(None);
    }
    let (_, _, mut file) = existing(root, path)?;
    let metadata = regular(&file)?;
    let expected_revision = revision(&metadata);
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(error)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    check_revision(&file, &expected_revision)?;
    if hex(&hash.finalize()) != expected_sha256 {
        return Ok(None);
    }
    Ok(Some(info(path, &metadata)))
}

fn with_upload_state(mut value: Value, unchanged: bool) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.insert("completed".into(), Value::Bool(true));
        object.insert("unchanged".into(), Value::Bool(unchanged));
    }
    value
}

fn publish(
    root: &OwnedFd,
    path: &str,
    temp: &Temporary,
    uid: u32,
    expected: Option<&str>,
) -> Result<Value, String> {
    publish_file(root, path, &temp.file, Some(&temp.parent), uid, expected)
}

fn publish_file(
    root: &OwnedFd,
    path: &str,
    file: &File,
    staged_in: Option<&OwnedFd>,
    uid: u32,
    expected: Option<&str>,
) -> Result<Value, String> {
    check_destination(root, path, expected)?;
    let (destination_parent, name) = parent(root, path)?;
    if let Some(staged_in) = staged_in {
        let a = sys::fstat(&destination_parent).map_err(error)?;
        let b = sys::fstat(staged_in).map_err(error)?;
        if a.st_dev != b.st_dev || a.st_ino != b.st_ino {
            return Err("destination directory moved during upload; abort and start again".into());
        }
    }
    let metadata = expected
        .map(|_| existing(root, path).and_then(|(_, _, file)| regular(&file)))
        .transpose()?;
    sys::fchown(
        file,
        Some(rustix::process::Uid::from_raw(
            metadata.as_ref().map_or(uid, Metadata::uid),
        )),
        Some(rustix::process::Gid::from_raw(
            metadata.as_ref().map_or(uid, Metadata::gid),
        )),
    )
    .map_err(error)?;
    sys::fchmod(
        file,
        Mode::from_raw_mode(metadata.as_ref().map_or(0o640, |m| m.mode() & 0o777)),
    )
    .map_err(error)?;
    file.sync_all().map_err(error)?;
    let source = format!("/proc/self/fd/{}", file.as_raw_fd());
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
        json!({"path": path, "revision": revision(&file.metadata().map_err(error)?), "recovery_path": recovery}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_privd::ServerFileRequest as R;
    use std::fs;
    use std::os::unix::fs::symlink;

    struct Fixture {
        root: tempfile::TempDir,
        state: tempfile::TempDir,
        files: ServerFiles,
    }
    impl Fixture {
        fn new() -> Self {
            Self {
                // Docker's overlay layer may lack O_TMPFILE; Linux tmpfs supports it.
                root: tempfile::tempdir_in("/dev/shm").unwrap(),
                state: tempfile::tempdir_in("/dev/shm").unwrap(),
                files: ServerFiles::default(),
            }
        }
        fn call(&self, request: R) -> Result<Value, String> {
            self.files.execute(
                "selected",
                self.root.path(),
                &self.pending_path(),
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

    impl Fixture {
        fn pending_path(&self) -> std::path::PathBuf {
            self.state.path().join("pending").join("selected")
        }
        fn stage(&self, request: R) -> Result<Value, String> {
            self.files.stage(
                "selected",
                self.root.path(),
                &self.pending_path(),
                rustix::process::getuid().as_raw(),
                request,
            )
        }
        fn pending(&self) -> Vec<PendingChange> {
            self.files.pending_changes(&self.pending_path()).unwrap()
        }
        fn apply(&self) -> AppliedPending {
            self.files
                .apply_pending(
                    self.root.path(),
                    &self.pending_path(),
                    rustix::process::getuid().as_raw(),
                )
                .unwrap()
        }
    }

    #[test]
    fn running_servers_stage_changes_until_they_are_applied() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.root.path().join("plugins")).unwrap();
        fs::write(fixture.root.path().join("plugins/Old.jar"), b"old jar").unwrap();
        fs::write(fixture.root.path().join("config.yml"), b"a: 1").unwrap();
        let config = fixture.revision("config.yml");
        let jar = fixture.revision("plugins/Old.jar");

        let staged = fixture
            .stage(R::Write {
                path: "config.yml".into(),
                content: "a: 2".into(),
                expected_revision: config.clone(),
            })
            .unwrap();
        assert_eq!(staged["pending"], true);
        // The live file is untouched until the change is applied.
        assert_eq!(
            fs::read(fixture.root.path().join("config.yml")).unwrap(),
            b"a: 1"
        );
        // Editing the staged file again uses its pending token.
        fixture
            .stage(R::Write {
                path: "config.yml".into(),
                content: "a: 3".into(),
                expected_revision: staged["revision"].as_str().unwrap().into(),
            })
            .unwrap();
        let bytes = b"new jar".to_vec();
        fixture
            .stage(R::UploadFile {
                path: "plugins/sub/New.jar".into(),
                data_base64: STANDARD.encode(&bytes),
                sha256: hex(&Sha256::digest(&bytes)),
                expected_revision: None,
            })
            .unwrap();
        fixture
            .stage(R::Trash {
                path: "plugins/Old.jar".into(),
                expected_revision: jar,
            })
            .unwrap();
        assert!(
            fixture
                .stage(R::Move {
                    path: "config.yml".into(),
                    destination: "other.yml".into(),
                    expected_revision: config.clone(),
                })
                .unwrap_err()
                .contains("stopped")
        );
        // Stale revisions are still refused.
        assert!(
            fixture
                .stage(R::Write {
                    path: "config.yml".into(),
                    content: "x".into(),
                    expected_revision: "stale".into(),
                })
                .is_err()
        );
        // Nothing appears in the live server while it runs, not even new folders.
        assert!(!fixture.root.path().join("plugins/sub").exists());
        let pending = fixture.pending();
        assert_eq!(pending.len(), 3);
        let listed = fixture.call(R::PendingList {}).unwrap();
        assert_eq!(listed["count"], 3);
        // The pending folder never shows up in listings.
        let root = fixture
            .call(R::List {
                path: String::new(),
                cursor: None,
                limit: 50,
            })
            .unwrap();
        assert!(
            !root["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["name"].as_str().unwrap().starts_with(".helix-api"))
        );

        let applied = fixture.apply();
        assert!(applied.failed.is_empty(), "{:?}", applied.failed);
        assert_eq!(applied.applied.len(), 3);
        assert_eq!(
            fs::read(fixture.root.path().join("config.yml")).unwrap(),
            b"a: 3"
        );
        assert_eq!(
            fs::read(fixture.root.path().join("plugins/sub/New.jar")).unwrap(),
            b"new jar"
        );
        assert!(!fixture.root.path().join("plugins/Old.jar").exists());
        assert!(fixture.pending().is_empty());
        let config_change = applied
            .applied
            .iter()
            .find(|change| change.path == "config.yml")
            .unwrap();
        assert!(!config_change.changed_since_staged);
        assert!(config_change.recovery_path.is_some());

        // Reverting puts every file back as it was.
        let errors = fixture
            .files
            .revert_applied(fixture.root.path(), &applied.applied);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            fs::read(fixture.root.path().join("config.yml")).unwrap(),
            b"a: 1"
        );
        assert_eq!(
            fs::read(fixture.root.path().join("plugins/Old.jar")).unwrap(),
            b"old jar"
        );
        assert!(!fixture.root.path().join("plugins/sub/New.jar").exists());
        assert!(!fixture.root.path().join("plugins/sub").exists());
    }

    #[test]
    fn a_forged_or_shared_pending_store_is_refused() {
        let fixture = Fixture::new();
        fixture
            .stage(R::Create {
                path: "a.txt".into(),
            })
            .unwrap();
        // Another staged new file at the same path must name the staged one.
        assert!(
            fixture
                .stage(R::Create {
                    path: "a.txt".into()
                })
                .unwrap_err()
                .contains("already staged")
        );
        // A store others can read is not trusted.
        fs::set_permissions(fixture.pending_path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            fixture
                .files
                .pending_changes(&fixture.pending_path())
                .unwrap_err()
                .contains("not private")
        );
        fs::set_permissions(fixture.pending_path(), fs::Permissions::from_mode(0o700)).unwrap();
        // An index naming a blob outside the store is rejected outright.
        fs::write(
            fixture.pending_path().join("pending.json"),
            br#"[{"path":"x","kind":"write","blob":"../../etc/passwd","size":1,"staged_at_unix_ms":1}]"#,
        )
        .unwrap();
        assert!(
            fixture
                .files
                .pending_changes(&fixture.pending_path())
                .unwrap_err()
                .contains("damaged")
        );
    }

    #[test]
    fn staged_uploads_discards_and_direct_writes_replace_pending_changes() {
        let fixture = Fixture::new();
        let bytes = vec![7_u8; CHUNK + 10];
        let begin = fixture
            .stage(R::UploadBegin {
                path: "big.bin".into(),
                size: bytes.len() as u64,
                sha256: hex(&Sha256::digest(&bytes)),
                expected_revision: None,
            })
            .unwrap();
        let id = begin["upload_id"].as_str().unwrap().to_owned();
        for (offset, chunk) in [(0, &bytes[..CHUNK]), (CHUNK, &bytes[CHUNK..])] {
            fixture
                .stage(R::UploadChunk {
                    upload_id: id.clone(),
                    offset: offset as u64,
                    data_base64: STANDARD.encode(chunk),
                })
                .unwrap();
        }
        let finished = fixture.stage(R::UploadFinish { upload_id: id }).unwrap();
        assert_eq!(finished["pending"], true);
        assert!(!fixture.root.path().join("big.bin").exists());
        assert_eq!(fixture.pending()[0].size, bytes.len() as u64);

        // Discarding drops the staged file.
        let discarded = fixture
            .call(R::PendingDiscard {
                path: Some("big.bin".into()),
            })
            .unwrap();
        assert_eq!(discarded["count"], 1);
        assert!(fixture.pending().is_empty());
        assert!(
            fixture
                .call(R::PendingDiscard {
                    path: Some("big.bin".into())
                })
                .is_err()
        );

        // An upload begun while running and finished after a stop stays staged.
        let begin = fixture
            .stage(R::UploadBegin {
                path: "late.jar".into(),
                size: 4,
                sha256: hex(&Sha256::digest(b"late")),
                expected_revision: None,
            })
            .unwrap();
        let id = begin["upload_id"].as_str().unwrap().to_owned();
        fixture
            .stage(R::UploadChunk {
                upload_id: id.clone(),
                offset: 0,
                data_base64: STANDARD.encode(b"late"),
            })
            .unwrap();
        assert_eq!(
            fixture.call(R::UploadFinish { upload_id: id }).unwrap()["pending"],
            true
        );
        assert_eq!(fixture.pending().len(), 1);
        fixture.call(R::PendingDiscard { path: None }).unwrap();

        // A direct chunked upload replaces a change staged for the same path.
        let begin = fixture
            .stage(R::UploadBegin {
                path: "plugin.jar".into(),
                size: 3,
                sha256: hex(&Sha256::digest(b"old")),
                expected_revision: None,
            })
            .unwrap();
        let id = begin["upload_id"].as_str().unwrap().to_owned();
        fixture
            .stage(R::UploadChunk {
                upload_id: id.clone(),
                offset: 0,
                data_base64: STANDARD.encode(b"old"),
            })
            .unwrap();
        fixture.stage(R::UploadFinish { upload_id: id }).unwrap();
        assert_eq!(fixture.pending().len(), 1);
        let id = fixture.begin("plugin.jar", b"new", None);
        fixture
            .call(R::UploadChunk {
                upload_id: id.clone(),
                offset: 0,
                data_base64: STANDARD.encode(b"new"),
            })
            .unwrap();
        fixture.call(R::UploadFinish { upload_id: id }).unwrap();
        assert!(fixture.pending().is_empty());
        assert_eq!(
            fs::read(fixture.root.path().join("plugin.jar")).unwrap(),
            b"new"
        );

        // A direct write to a stopped server wins over a staged change.
        fixture
            .stage(R::Create {
                path: "notes.txt".into(),
            })
            .unwrap();
        assert_eq!(fixture.pending().len(), 1);
        fixture
            .call(R::Create {
                path: "notes.txt".into(),
            })
            .unwrap();
        assert!(fixture.pending().is_empty());
        let applied = fixture.apply();
        assert!(applied.applied.is_empty());
    }

    #[test]
    fn a_failed_install_keeps_every_change_staged() {
        let fixture = Fixture::new();
        fs::write(fixture.root.path().join("a.txt"), b"old a").unwrap();
        let a = fixture.revision("a.txt");
        fixture
            .stage(R::Write {
                path: "a.txt".into(),
                content: "new a".into(),
                expected_revision: a,
            })
            .unwrap();
        fixture
            .stage(R::Create {
                path: "dir/b.txt".into(),
            })
            .unwrap();
        // A folder now sits where the staged file should go.
        fs::create_dir_all(fixture.root.path().join("dir/b.txt")).unwrap();
        let result = fixture.apply();
        assert_eq!(result.failed.len(), 1);
        let errors = fixture
            .files
            .revert_applied(fixture.root.path(), &result.applied);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            fs::read(fixture.root.path().join("a.txt")).unwrap(),
            b"old a"
        );
        assert_eq!(fixture.pending().len(), 2);
        // Once the conflict is gone, the same staged changes install.
        fs::remove_dir(fixture.root.path().join("dir/b.txt")).unwrap();
        let result = fixture.apply();
        assert!(result.failed.is_empty(), "{:?}", result.failed);
        assert_eq!(
            fs::read(fixture.root.path().join("a.txt")).unwrap(),
            b"new a"
        );
        assert!(fixture.root.path().join("dir/b.txt").is_file());
        assert!(fixture.pending().is_empty());
    }

    #[test]
    fn os_errors_become_plain_sentences() {
        let missing = std::io::Error::from_raw_os_error(2);
        assert!(error(missing).contains("does not exist"));
        assert!(error(rustix::io::Errno::NOSPC).contains("disk is full"));
        assert!(error(rustix::io::Errno::EXIST).contains("already exists"));
        assert!(error("something else").contains("server file operation failed"));
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
                    &f.pending_path(),
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
        let replay = f
            .call(R::UploadChunk {
                upload_id: id.clone(),
                offset: 0,
                data_base64: STANDARD.encode(b"payload"),
            })
            .unwrap();
        assert_eq!(replay["bytes_written"], 7);
        assert_eq!(replay["replayed"], true);
        assert!(
            f.call(R::UploadChunk {
                upload_id: id.clone(),
                offset: 0,
                data_base64: STANDARD.encode(b"different")
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

    #[test]
    fn upload_begin_resumes_matching_work_and_replaces_abandoned_work() {
        let f = Fixture::new();
        let first = f.begin("plugin.jar", b"payload", None);
        f.call(R::UploadChunk {
            upload_id: first.clone(),
            offset: 0,
            data_base64: STANDARD.encode(b"pay"),
        })
        .unwrap();
        let resumed = f
            .call(R::UploadBegin {
                path: "plugin.jar".into(),
                size: 7,
                sha256: hex(&Sha256::digest(b"payload")),
                expected_revision: None,
            })
            .unwrap();
        assert_eq!(resumed["upload_id"], first);
        assert_eq!(resumed["bytes_written"], 3);
        assert_eq!(resumed["resumed"], true);

        let replacement = f
            .call(R::UploadBegin {
                path: "plugin.jar".into(),
                size: 3,
                sha256: hex(&Sha256::digest(b"new")),
                expected_revision: None,
            })
            .unwrap();
        assert_ne!(replacement["upload_id"], first);
        assert_eq!(replacement["bytes_written"], 0);
        assert_eq!(replacement["resumed"], false);
        assert!(f.call(R::UploadStatus { upload_id: first }).is_err());
    }

    #[test]
    fn direct_upload_and_lost_finish_retries_are_idempotent() {
        let f = Fixture::new();
        let payload = b"plugin";
        let digest = hex(&Sha256::digest(payload));
        assert!(
            f.call(R::UploadFile {
                path: "plugin.jar".into(),
                data_base64: STANDARD.encode(payload),
                sha256: hex(&Sha256::digest(b"different")),
                expected_revision: None,
            })
            .is_err()
        );
        assert!(!f.root.path().join("plugin.jar").exists());
        let first = f
            .call(R::UploadFile {
                path: "plugin.jar".into(),
                data_base64: STANDARD.encode(payload),
                sha256: digest.clone(),
                expected_revision: None,
            })
            .unwrap();
        assert_eq!(first["completed"], true);
        assert_eq!(first["unchanged"], false);
        let revision = first["revision"].as_str().unwrap().to_owned();

        let retry = f
            .call(R::UploadFile {
                path: "plugin.jar".into(),
                data_base64: STANDARD.encode(payload),
                sha256: digest.clone(),
                expected_revision: None,
            })
            .unwrap();
        assert_eq!(retry["revision"], revision);
        assert_eq!(retry["unchanged"], true);

        let replacement = b"replacement";
        let old_revision = f.revision("plugin.jar");
        let finished = f
            .call(R::UploadFile {
                path: "plugin.jar".into(),
                data_base64: STANDARD.encode(replacement),
                sha256: hex(&Sha256::digest(replacement)),
                expected_revision: Some(old_revision.clone()),
            })
            .unwrap();
        assert_eq!(finished["unchanged"], false);
        let lost_response_retry = f
            .call(R::UploadFile {
                path: "plugin.jar".into(),
                data_base64: STANDARD.encode(replacement),
                sha256: hex(&Sha256::digest(replacement)),
                expected_revision: Some(old_revision),
            })
            .unwrap();
        assert_eq!(lost_response_retry["unchanged"], true);
        assert_eq!(
            fs::read(f.root.path().join("plugin.jar")).unwrap(),
            replacement
        );
    }

    #[test]
    fn upload_begin_recognizes_a_completed_publish_after_response_loss() {
        let f = Fixture::new();
        let payload = b"large payload";
        let digest = hex(&Sha256::digest(payload));
        let id = f.begin("mod.jar", payload, None);
        f.call(R::UploadChunk {
            upload_id: id.clone(),
            offset: 0,
            data_base64: STANDARD.encode(payload),
        })
        .unwrap();
        f.call(R::UploadFinish { upload_id: id }).unwrap();

        let retry = f
            .call(R::UploadBegin {
                path: "mod.jar".into(),
                size: payload.len() as u64,
                sha256: digest,
                expected_revision: None,
            })
            .unwrap();
        assert_eq!(retry["completed"], true);
        assert_eq!(retry["unchanged"], true);
        assert!(retry.get("upload_id").is_none());
    }
}
