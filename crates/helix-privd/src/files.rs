use base64::{Engine as _, engine::general_purpose::STANDARD};
use helix_privd::{
    FileUploadPurpose, MAX_CONCURRENT_FILE_UPLOADS, MAX_CUSTOM_JAR_UPLOAD_BYTES,
    MAX_FILE_UPLOAD_CHUNK_BYTES, MAX_STORAGE_UPLOAD_BYTES, StorageAnalysisMode,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, HashMap, HashSet},
    ffi::{CStr, CString, OsStr},
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    os::fd::OwnedFd,
    os::unix::ffi::OsStrExt as _,
    os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _, chown},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const MIN_DIRECTORY_PAGE_ENTRIES: u16 = 25;
const MAX_DIRECTORY_PAGE_ENTRIES: u16 = 200;
const MAX_TEXT_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_CONFIGURED_ROOTS: usize = 32;
const BLOCKED_ROOTS: &[&str] = &["/proc", "/sys", "/dev", "/run"];
const BLOCKED_FILES: &[&str] = &[
    "/etc/shadow",
    "/etc/gshadow",
    "/etc/security/opasswd",
    "/root/.ssh",
];
const MIN_CUSTOM_JAR_BYTES: u64 = 16 * 1024;
const FILE_UPLOAD_IDLE: Duration = Duration::from_secs(10 * 60);
const JAR_MAGIC: &[u8] = b"PK\x03\x04";

#[derive(Clone)]
pub struct FileManager {
    managed_roots: Vec<PathBuf>,
    uploads: Arc<Mutex<HashMap<String, StreamingUpload>>>,
}

#[derive(Debug, Serialize)]
pub struct DirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub writable: bool,
    pub entries: Vec<FileEntry>,
    pub omitted_entries: usize,
    pub total_entries: usize,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub page_limit: u16,
}

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub kind: FileKind,
    pub size_bytes: u64,
    pub modified_unix_ms: Option<u64>,
    pub permissions: String,
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub writable: bool,
    pub restricted: bool,
    pub symlink_target: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Serialize)]
pub struct TextFile {
    pub path: String,
    pub content: String,
    pub size_bytes: u64,
    pub modified_unix_ms: Option<u64>,
}

impl FileManager {
    pub fn new(roots: Vec<PathBuf>) -> Result<Self, String> {
        if roots.is_empty() {
            return Err("at least one managed root is required".to_owned());
        }
        if roots.len() > MAX_CONFIGURED_ROOTS {
            return Err("too many managed roots are configured".to_owned());
        }
        let mut managed_roots = roots
            .into_iter()
            .map(|root| {
                fs::canonicalize(&root)
                    .map_err(|_| format!("managed root {} is unavailable", root.display()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        managed_roots.sort();
        managed_roots.dedup();
        managed_roots.sort_by_key(|right| std::cmp::Reverse(right.components().count()));
        Ok(Self {
            managed_roots,
            uploads: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn list(
        &self,
        path: &str,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<DirectoryListing, String> {
        if !(MIN_DIRECTORY_PAGE_ENTRIES..=MAX_DIRECTORY_PAGE_ENTRIES).contains(&limit) {
            return Err("the directory page size must be between 25 and 200 entries".to_owned());
        }
        if cursor.is_some_and(|cursor| {
            cursor.is_empty()
                || cursor.len() > 1_024
                || cursor.chars().any(char::is_control)
                || matches!(cursor, "." | "..")
        }) {
            return Err("the directory cursor is invalid".to_owned());
        }
        let canonical = canonical_existing(path)?;
        ensure_visible(&canonical)?;
        if !canonical.is_dir() {
            return Err("the selected path is not a directory".to_owned());
        }
        let cursor_key = cursor.map(directory_name_key);
        let mut total_entries = 0_usize;
        let mut omitted_entries = 0_usize;
        let mut candidates = BinaryHeap::new();
        let candidate_limit = usize::from(limit).saturating_add(1);
        let reader = fs::read_dir(&canonical).map_err(file_error)?;
        for item in reader {
            let item = match item {
                Ok(item) => item,
                Err(_) => {
                    omitted_entries = omitted_entries.saturating_add(1);
                    continue;
                }
            };
            let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                omitted_entries = omitted_entries.saturating_add(1);
                continue;
            };
            if name.len() > 1024 || name.chars().any(char::is_control) {
                omitted_entries = omitted_entries.saturating_add(1);
                continue;
            }
            if name.starts_with(".helix-upload-") {
                continue;
            }
            total_entries = total_entries.saturating_add(1);
            let key = directory_name_key(&name);
            if cursor_key.as_ref().is_some_and(|cursor| &key <= cursor) {
                continue;
            }
            candidates.push((key, name));
            if candidates.len() > candidate_limit {
                candidates.pop();
            }
        }
        let mut candidate_names = candidates.into_sorted_vec();
        let has_more = candidate_names.len() > usize::from(limit);
        if has_more {
            candidate_names.pop();
        }
        let next_cursor = has_more
            .then(|| candidate_names.last().map(|(_, name)| name.clone()))
            .flatten();
        let mut entries = Vec::with_capacity(candidate_names.len());
        for (_, name) in candidate_names {
            let item_path = canonical.join(&name);
            let metadata = match fs::symlink_metadata(&item_path) {
                Ok(metadata) => metadata,
                Err(_) => {
                    omitted_entries = omitted_entries.saturating_add(1);
                    continue;
                }
            };
            let file_type = metadata.file_type();
            let kind = if file_type.is_symlink() {
                FileKind::Symlink
            } else if file_type.is_dir() {
                FileKind::Directory
            } else if file_type.is_file() {
                FileKind::File
            } else {
                FileKind::Other
            };
            let restricted = is_blocked(&item_path) || file_type.is_symlink();
            entries.push(FileEntry {
                name,
                path: item_path.to_string_lossy().into_owned(),
                kind,
                size_bytes: metadata.len(),
                modified_unix_ms: modified_unix_ms(&metadata),
                permissions: format!("{:04o}", metadata.permissions().mode() & 0o7777),
                owner_uid: metadata.uid(),
                owner_gid: metadata.gid(),
                writable: !restricted && self.managed_root_for(&item_path).is_some(),
                restricted,
                symlink_target: file_type
                    .is_symlink()
                    .then(|| fs::read_link(&item_path).ok())
                    .flatten()
                    .map(|target| target.to_string_lossy().into_owned()),
            });
        }
        Ok(DirectoryListing {
            path: canonical.to_string_lossy().into_owned(),
            parent: canonical
                .parent()
                .map(|parent| parent.to_string_lossy().into_owned()),
            writable: self.managed_root_for(&canonical).is_some(),
            omitted_entries,
            total_entries,
            next_cursor,
            has_more,
            page_limit: limit,
            entries,
        })
    }

    pub fn create_directory(&self, parent: &str, name: &str) -> Result<PathResult, String> {
        let (parent, target) = self.new_target(parent, name)?;
        fs::create_dir(&target).map_err(file_error)?;
        inherit_owner_and_directory_mode(&parent, &target)?;
        sync_directory(&parent)?;
        Ok(PathResult::new(target))
    }

    pub fn create_file(&self, parent: &str, name: &str) -> Result<PathResult, String> {
        let (parent, target) = self.new_target(parent, name)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o640)
            .open(&target)
            .map_err(file_error)?;
        file.sync_all().map_err(file_error)?;
        inherit_owner(&parent, &target)?;
        sync_directory(&parent)?;
        Ok(PathResult::new(target))
    }

    pub fn read_text(&self, path: &str) -> Result<TextFile, String> {
        let canonical = canonical_existing(path)?;
        ensure_visible(&canonical)?;
        let metadata = fs::symlink_metadata(&canonical).map_err(file_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("the selected path is not a regular file".to_owned());
        }
        if metadata.len() > MAX_TEXT_BYTES {
            return Err("the text editor is limited to 4 MiB UTF-8 files".to_owned());
        }
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        File::open(&canonical)
            .and_then(|file| file.take(MAX_TEXT_BYTES + 1).read_to_end(&mut bytes))
            .map_err(file_error)?;
        if bytes.len() as u64 > MAX_TEXT_BYTES {
            return Err("the text editor is limited to 4 MiB UTF-8 files".to_owned());
        }
        let content = String::from_utf8(bytes)
            .map_err(|_| "the selected file is not UTF-8 text".to_owned())?;
        Ok(TextFile {
            path: canonical.to_string_lossy().into_owned(),
            content,
            size_bytes: metadata.len(),
            modified_unix_ms: modified_unix_ms(&metadata),
        })
    }

    pub fn write_text(
        &self,
        path: &str,
        content: &str,
        expected_modified_unix_ms: Option<u64>,
    ) -> Result<TextFile, String> {
        if content.len() as u64 > MAX_TEXT_BYTES {
            return Err("the text editor is limited to 4 MiB UTF-8 files".to_owned());
        }
        let canonical = self.mutable_existing(path)?;
        let metadata = fs::symlink_metadata(&canonical).map_err(file_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("the selected path is not a regular file".to_owned());
        }
        if expected_modified_unix_ms.is_some()
            && expected_modified_unix_ms != modified_unix_ms(&metadata)
        {
            return Err("the file changed after it was opened; reload before saving".to_owned());
        }
        let parent = canonical
            .parent()
            .ok_or_else(|| "the file has no parent directory".to_owned())?;
        let temporary = parent.join(format!(".helix-write-{}", Uuid::new_v4().simple()));
        let write_result = (|| -> Result<(), String> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(metadata.permissions().mode() & 0o7777)
                .open(&temporary)
                .map_err(file_error)?;
            file.write_all(content.as_bytes()).map_err(file_error)?;
            file.sync_all().map_err(file_error)?;
            fs::set_permissions(&temporary, metadata.permissions()).map_err(file_error)?;
            chown(&temporary, Some(metadata.uid()), Some(metadata.gid())).map_err(file_error)?;
            fs::rename(&temporary, &canonical).map_err(file_error)?;
            sync_directory(parent)
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result?;
        self.read_text(canonical.to_string_lossy().as_ref())
    }

    pub fn rename(&self, path: &str, new_name: &str) -> Result<PathResult, String> {
        validate_name(new_name)?;
        let canonical = self.mutable_existing(path)?;
        let metadata = fs::symlink_metadata(&canonical).map_err(file_error)?;
        if metadata.file_type().is_symlink() {
            return Err("symbolic links cannot be renamed from Helix".to_owned());
        }
        let parent = canonical
            .parent()
            .ok_or_else(|| "the selected path cannot be renamed".to_owned())?;
        let target = parent.join(new_name);
        ensure_absent(&target)?;
        fs::rename(&canonical, &target).map_err(file_error)?;
        sync_directory(parent)?;
        Ok(PathResult::new(target))
    }

    pub fn trash(&self, path: &str) -> Result<TrashResult, String> {
        let canonical = self.mutable_existing(path)?;
        let metadata = fs::symlink_metadata(&canonical).map_err(file_error)?;
        if metadata.file_type().is_symlink() {
            return Err("symbolic links cannot be removed from Helix".to_owned());
        }
        let managed_root = self
            .managed_root_for(&canonical)
            .ok_or_else(|| "the selected path is outside a managed storage root".to_owned())?;
        if canonical == *managed_root {
            return Err("a managed storage root cannot be removed".to_owned());
        }
        let trash_root = managed_root.join(".helix-trash");
        if !trash_root.exists() {
            fs::create_dir(&trash_root).map_err(file_error)?;
            inherit_owner_and_directory_mode(managed_root, &trash_root)?;
        }
        let name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("item");
        let target = trash_root.join(format!(
            "{}-{}-{}",
            now_unix_ms(),
            Uuid::new_v4().simple(),
            name
        ));
        fs::rename(&canonical, &target).map_err(file_error)?;
        sync_directory(canonical.parent().unwrap_or(managed_root))?;
        sync_directory(&trash_root)?;
        Ok(TrashResult {
            original_path: canonical.to_string_lossy().into_owned(),
            recovery_path: target.to_string_lossy().into_owned(),
        })
    }

    pub fn begin_upload(
        &self,
        parent: &str,
        name: &str,
        expected_size: u64,
    ) -> Result<FileUploadStart, String> {
        if expected_size == 0 || expected_size > MAX_STORAGE_UPLOAD_BYTES {
            return Err("uploaded files must be between 1 byte and 256 MiB".to_owned());
        }
        let (parent, destination) = self.new_target(parent, name)?;
        let upload = StreamingUpload::start(
            parent,
            destination,
            expected_size,
            FileUploadPurpose::Storage,
        )?;
        self.insert_upload(upload)
    }

    pub fn write_upload_chunk(
        &self,
        upload_id: &str,
        purpose: FileUploadPurpose,
        offset: u64,
        data_base64: &str,
    ) -> Result<FileUploadProgress, String> {
        if purpose != FileUploadPurpose::Storage {
            return Err("that upload does not belong to Storage".to_owned());
        }
        let data = decode_upload_chunk(data_base64)?;
        let mut uploads = self.uploads.lock().map_err(|_| upload_lock_error())?;
        prune_uploads(&mut uploads);
        let upload = uploads
            .get_mut(upload_id)
            .ok_or_else(|| "that upload is no longer active".to_owned())?;
        if upload.purpose != purpose {
            return Err("that upload does not belong to Storage".to_owned());
        }
        let written = upload.write_chunk(offset, &data)?;
        Ok(FileUploadProgress {
            upload_id: upload_id.to_owned(),
            bytes_written: written,
            expected_size: upload.expected_size,
        })
    }

    pub fn finish_upload(
        &self,
        upload_id: &str,
        purpose: FileUploadPurpose,
    ) -> Result<PathResult, String> {
        if purpose != FileUploadPurpose::Storage {
            return Err("that upload does not belong to Storage".to_owned());
        }
        let mut uploads = self.uploads.lock().map_err(|_| upload_lock_error())?;
        prune_uploads(&mut uploads);
        let upload = uploads
            .remove(upload_id)
            .ok_or_else(|| "that upload is no longer active".to_owned())?;
        if upload.purpose != purpose {
            uploads.insert(upload_id.to_owned(), upload);
            return Err("that upload does not belong to Storage".to_owned());
        }
        Ok(PathResult::new(commit_upload(upload)?))
    }

    pub fn abort_upload(
        &self,
        upload_id: &str,
        purpose: FileUploadPurpose,
    ) -> Result<Value, String> {
        if purpose != FileUploadPurpose::Storage {
            return Err("that upload does not belong to Storage".to_owned());
        }
        let mut uploads = self.uploads.lock().map_err(|_| upload_lock_error())?;
        prune_uploads(&mut uploads);
        if let Some(upload) = uploads.remove(upload_id) {
            if upload.purpose != purpose {
                uploads.insert(upload_id.to_owned(), upload);
                return Err("that upload does not belong to Storage".to_owned());
            }
            upload.abort();
        }
        Ok(json!({ "aborted": true }))
    }

    fn insert_upload(&self, upload: StreamingUpload) -> Result<FileUploadStart, String> {
        let mut uploads = self.uploads.lock().map_err(|_| upload_lock_error())?;
        prune_uploads(&mut uploads);
        if uploads.len() >= MAX_CONCURRENT_FILE_UPLOADS {
            upload.abort();
            return Err("Helix is already receiving the maximum number of uploads".to_owned());
        }
        if uploads
            .values()
            .any(|existing| existing.destination == upload.destination)
        {
            upload.abort();
            return Err("an item with that name already exists".to_owned());
        }
        let upload_id = Uuid::new_v4().to_string();
        let start = FileUploadStart {
            upload_id: upload_id.clone(),
            expected_size: upload.expected_size,
            max_chunk_bytes: MAX_FILE_UPLOAD_CHUNK_BYTES as u64,
            purpose: upload.purpose,
        };
        uploads.insert(upload_id, upload);
        Ok(start)
    }

    fn new_target(&self, parent: &str, name: &str) -> Result<(PathBuf, PathBuf), String> {
        validate_name(name)?;
        let parent = canonical_existing(parent)?;
        ensure_visible(&parent)?;
        if !parent.is_dir() || self.managed_root_for(&parent).is_none() {
            return Err("the selected directory is not writable through Helix".to_owned());
        }
        let target = parent.join(name);
        ensure_absent(&target)?;
        Ok((parent, target))
    }

    fn mutable_existing(&self, path: &str) -> Result<PathBuf, String> {
        let input = Path::new(path);
        let metadata = fs::symlink_metadata(input).map_err(file_error)?;
        if metadata.file_type().is_symlink() {
            return Err("symbolic links cannot be changed from Helix".to_owned());
        }
        let canonical = canonical_existing(path)?;
        ensure_visible(&canonical)?;
        if self.managed_root_for(&canonical).is_none() {
            return Err("the selected path is outside a managed storage root".to_owned());
        }
        Ok(canonical)
    }

    fn managed_root_for(&self, path: &Path) -> Option<&PathBuf> {
        self.managed_roots
            .iter()
            .find(|root| path.starts_with(root))
    }
}

#[derive(Debug, Serialize)]
pub struct PathResult {
    pub path: String,
}

impl PathResult {
    fn new(path: PathBuf) -> Self {
        Self {
            path: path.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FileUploadStart {
    pub upload_id: String,
    pub expected_size: u64,
    pub max_chunk_bytes: u64,
    pub purpose: FileUploadPurpose,
}

#[derive(Debug, Serialize)]
pub struct FileUploadProgress {
    pub upload_id: String,
    pub bytes_written: u64,
    pub expected_size: u64,
}

pub(crate) struct StreamingUpload {
    pub(crate) destination: PathBuf,
    temporary: PathBuf,
    parent: PathBuf,
    pub(crate) expected_size: u64,
    written: u64,
    file: File,
    touched: Instant,
    pub(crate) purpose: FileUploadPurpose,
    finished: bool,
}

impl StreamingUpload {
    pub(crate) fn start(
        parent: PathBuf,
        destination: PathBuf,
        expected_size: u64,
        purpose: FileUploadPurpose,
    ) -> Result<Self, String> {
        let temporary = parent.join(format!(".helix-upload-{}", Uuid::new_v4().simple()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o640)
            .open(&temporary)
            .map_err(file_error)?;
        Ok(Self {
            destination,
            temporary,
            parent,
            expected_size,
            written: 0,
            file,
            touched: Instant::now(),
            purpose,
            finished: false,
        })
    }

    pub(crate) fn is_stale(&self) -> bool {
        self.touched.elapsed() > FILE_UPLOAD_IDLE
    }

    pub(crate) fn write_chunk(&mut self, offset: u64, data: &[u8]) -> Result<u64, String> {
        if data.is_empty() {
            return Err("upload chunks cannot be empty".to_owned());
        }
        if offset != self.written {
            return Err("upload chunks must arrive in order".to_owned());
        }
        let next = self
            .written
            .checked_add(data.len() as u64)
            .ok_or_else(|| "the upload is too large".to_owned())?;
        if next > self.expected_size {
            return Err("the upload exceeded its declared size".to_owned());
        }
        if self.purpose == FileUploadPurpose::CustomJar
            && self.written == 0
            && (data.len() < JAR_MAGIC.len() || !data.starts_with(JAR_MAGIC))
        {
            return Err("the dropped file is not a JAR archive".to_owned());
        }
        self.file.write_all(data).map_err(file_error)?;
        self.written = next;
        self.touched = Instant::now();
        Ok(self.written)
    }

    fn finish(mut self) -> Result<PathBuf, String> {
        if self.written != self.expected_size {
            return Err("the upload ended before the declared size was received".to_owned());
        }
        if self.purpose == FileUploadPurpose::CustomJar && self.written < MIN_CUSTOM_JAR_BYTES {
            return Err("the custom server JAR is smaller than the 16 KiB minimum".to_owned());
        }
        self.file.sync_all().map_err(file_error)?;
        fs::hard_link(&self.temporary, &self.destination).map_err(file_error)?;
        self.finished = true;
        let _ = fs::remove_file(&self.temporary);
        Ok(self.destination.clone())
    }

    pub(crate) fn abort(mut self) {
        self.finished = true;
        let _ = fs::remove_file(&self.temporary);
    }
}

impl Drop for StreamingUpload {
    fn drop(&mut self) {
        if !self.finished {
            let _ = fs::remove_file(&self.temporary);
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TrashResult {
    pub original_path: String,
    pub recovery_path: String,
}

pub const STORAGE_ANALYSIS_MAX_DURATION: Duration = Duration::from_secs(30);
pub const STORAGE_ANALYSIS_MAX_ENTRIES: u64 = 250_000;
pub const STORAGE_ANALYSIS_MAX_DEPTH: u16 = 64;
pub const STORAGE_ANALYSIS_MAX_RESULTS_PER_LIST: usize = 128;
pub const STORAGE_ANALYSIS_MAX_CONCURRENT_JOBS: usize = 2;
pub const STORAGE_ANALYSIS_MAX_RETAINED_JOBS: usize = 32;
pub const STORAGE_ANALYSIS_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const STORAGE_ANALYSIS_THOROUGH_MAX_DURATION: Duration = Duration::from_secs(10 * 60);
pub const STORAGE_ANALYSIS_THOROUGH_MAX_ENTRIES: u64 = 5_000_000;
pub const STORAGE_ANALYSIS_THOROUGH_MAX_DEPTH: u16 = 128;
pub const STORAGE_ANALYSIS_THOROUGH_MAX_RESULTS_PER_LIST: usize = 2_048;
pub const STORAGE_ANALYSIS_THOROUGH_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

const STORAGE_ANALYSIS_PROGRESS_INTERVAL: u64 = 256;
const STORAGE_ANALYSIS_READ_BUFFER_BYTES: usize = 32 * 1024;

#[derive(Clone)]
pub struct StorageAnalysisManager {
    roots: Arc<Vec<StorageAnalysisRoot>>,
    jobs: Arc<StorageAnalysisJobs>,
    quick_limits: StorageAnalysisLimits,
    thorough_limits: StorageAnalysisLimits,
}

struct StorageAnalysisRoot {
    path: PathBuf,
    descriptor: Arc<OwnedFd>,
}

struct StorageAnalysisJobs {
    inner: Mutex<StorageAnalysisJobRegistry>,
}

#[derive(Default)]
struct StorageAnalysisJobRegistry {
    active: usize,
    jobs: HashMap<Uuid, StorageAnalysisJobRecord>,
}

struct StorageAnalysisJobRecord {
    id: Uuid,
    requested_path: String,
    state: StorageAnalysisJobState,
    progress: StorageAnalysisProgress,
    created_unix_ms: u64,
    started_unix_ms: Option<u64>,
    finished_unix_ms: Option<u64>,
    cancel_requested: bool,
    cancellation: Arc<AtomicBool>,
    result: Option<StorageAnalysisResult>,
    error: Option<String>,
}

#[derive(Clone, Copy)]
struct StorageAnalysisLimits {
    max_duration: Duration,
    max_entries: u64,
    max_depth: u16,
    max_results_per_list: usize,
    max_concurrent_jobs: usize,
    max_retained_jobs: usize,
    max_response_bytes: usize,
    progress_interval: u64,
    per_entry_delay: Duration,
}

impl Default for StorageAnalysisLimits {
    fn default() -> Self {
        Self {
            max_duration: STORAGE_ANALYSIS_MAX_DURATION,
            max_entries: STORAGE_ANALYSIS_MAX_ENTRIES,
            max_depth: STORAGE_ANALYSIS_MAX_DEPTH,
            max_results_per_list: STORAGE_ANALYSIS_MAX_RESULTS_PER_LIST,
            max_concurrent_jobs: STORAGE_ANALYSIS_MAX_CONCURRENT_JOBS,
            max_retained_jobs: STORAGE_ANALYSIS_MAX_RETAINED_JOBS,
            max_response_bytes: STORAGE_ANALYSIS_MAX_RESPONSE_BYTES,
            progress_interval: STORAGE_ANALYSIS_PROGRESS_INTERVAL,
            per_entry_delay: Duration::ZERO,
        }
    }
}

impl StorageAnalysisLimits {
    fn thorough() -> Self {
        Self {
            max_duration: STORAGE_ANALYSIS_THOROUGH_MAX_DURATION,
            max_entries: STORAGE_ANALYSIS_THOROUGH_MAX_ENTRIES,
            max_depth: STORAGE_ANALYSIS_THOROUGH_MAX_DEPTH,
            max_results_per_list: STORAGE_ANALYSIS_THOROUGH_MAX_RESULTS_PER_LIST,
            max_concurrent_jobs: 1,
            max_retained_jobs: STORAGE_ANALYSIS_MAX_RETAINED_JOBS,
            max_response_bytes: STORAGE_ANALYSIS_THOROUGH_MAX_RESPONSE_BYTES,
            progress_interval: STORAGE_ANALYSIS_PROGRESS_INTERVAL,
            per_entry_delay: Duration::ZERO,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAnalysisJobState {
    Queued,
    Running,
    Complete,
    Cancelled,
    Failed,
}

impl StorageAnalysisJobState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Complete | Self::Cancelled | Self::Failed)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StorageAnalysisStart {
    pub job_id: String,
    pub state: StorageAnalysisJobState,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StorageAnalysisProgress {
    pub percent: Option<u8>,
    pub entry_budget_percent: u8,
    pub duration_budget_percent: u8,
    pub entries_scanned: u64,
    pub files_scanned: u64,
    pub directories_scanned: u64,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub bytes_scanned: u64,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub allocated_bytes_scanned: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct StorageAnalysisStatus {
    pub job_id: String,
    pub requested_path: String,
    pub state: StorageAnalysisJobState,
    pub progress: StorageAnalysisProgress,
    pub created_unix_ms: u64,
    pub started_unix_ms: Option<u64>,
    pub finished_unix_ms: Option<u64>,
    pub cancel_requested: bool,
    pub result: Option<StorageAnalysisResult>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAnalysisEntryKind {
    File,
    Directory,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StorageAnalysisFile {
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: StorageAnalysisEntryKind,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub bytes: u64,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub allocated_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StorageAnalysisFolder {
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: StorageAnalysisEntryKind,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub immediate_bytes: u64,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub recursive_bytes: u64,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub immediate_allocated_bytes: u64,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub recursive_allocated_bytes: u64,
    pub immediate_complete: bool,
    pub recursive_complete: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StorageAnalysisErrors {
    pub total: u64,
    pub permission_denied: u64,
    pub filesystem_races: u64,
    pub metadata_failures: u64,
    pub size_overflows: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StorageAnalysisSkipped {
    pub restricted_paths: u64,
    pub symbolic_links: u64,
    pub other_filesystems: u64,
    pub special_files: u64,
    pub depth_limited_directories: u64,
    pub unrepresentable_names: u64,
    pub hard_link_aliases: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageAnalysisStopReason {
    Cancelled,
    DurationLimit,
    EntryLimit,
}

#[derive(Clone, Debug, Serialize)]
pub struct StorageAnalysisAppliedLimits {
    pub mode: StorageAnalysisMode,
    pub max_duration_ms: u64,
    pub max_entries: u64,
    pub max_depth: u16,
    pub max_results_per_list: usize,
    pub max_response_bytes: usize,
    pub stay_on_target_filesystem: bool,
    pub follows_symbolic_links: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct StorageAnalysisResult {
    pub root_path: String,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub apparent_bytes_scanned: u64,
    #[serde(serialize_with = "serialize_u64_decimal")]
    pub allocated_bytes_scanned: u64,
    pub truncated: bool,
    pub stop_reason: Option<StorageAnalysisStopReason>,
    pub errors: StorageAnalysisErrors,
    pub skipped: StorageAnalysisSkipped,
    pub file_results_omitted: u64,
    pub recursive_folder_results_omitted: u64,
    pub immediate_folder_results_omitted: u64,
    pub response_results_omitted: u64,
    pub largest_files: Vec<StorageAnalysisFile>,
    pub largest_folders_by_recursive_bytes: Vec<StorageAnalysisFolder>,
    pub largest_folders_by_immediate_bytes: Vec<StorageAnalysisFolder>,
    pub limits: StorageAnalysisAppliedLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RankedFile(StorageAnalysisFile);

impl Ord for RankedFile {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .allocated_bytes
            .cmp(&other.0.allocated_bytes)
            .then_with(|| self.0.bytes.cmp(&other.0.bytes))
            .then_with(|| self.0.path.cmp(&other.0.path))
    }
}

impl PartialOrd for RankedFile {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RankedFolder {
    bytes: u64,
    folder: StorageAnalysisFolder,
}

impl Ord for RankedFolder {
    fn cmp(&self, other: &Self) -> Ordering {
        self.bytes
            .cmp(&other.bytes)
            .then_with(|| self.folder.path.cmp(&other.folder.path))
    }
}

impl PartialOrd for RankedFolder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct StorageScan {
    job_id: Uuid,
    jobs: Arc<StorageAnalysisJobs>,
    cancellation: Arc<AtomicBool>,
    started: Instant,
    target_device: rustix::fs::Dev,
    mode: StorageAnalysisMode,
    limits: StorageAnalysisLimits,
    progress: StorageAnalysisProgress,
    errors: StorageAnalysisErrors,
    skipped: StorageAnalysisSkipped,
    stop_reason: Option<StorageAnalysisStopReason>,
    files_seen: u64,
    folders_seen: u64,
    largest_files: BinaryHeap<Reverse<RankedFile>>,
    recursive_folders: BinaryHeap<Reverse<RankedFolder>>,
    immediate_folders: BinaryHeap<Reverse<RankedFolder>>,
    hard_link_inodes: HashSet<(u64, u64)>,
}

#[derive(Clone, Copy)]
struct DirectoryTotals {
    immediate_bytes: u64,
    recursive_bytes: u64,
    immediate_allocated_bytes: u64,
    recursive_allocated_bytes: u64,
    immediate_complete: bool,
    recursive_complete: bool,
}

impl StorageAnalysisManager {
    pub fn new(roots: Vec<PathBuf>) -> Result<Self, String> {
        Self::with_limit_profiles(
            roots,
            StorageAnalysisLimits::default(),
            StorageAnalysisLimits::thorough(),
        )
    }

    #[cfg(test)]
    fn with_limits(roots: Vec<PathBuf>, limits: StorageAnalysisLimits) -> Result<Self, String> {
        Self::with_limit_profiles(roots, limits, limits)
    }

    fn with_limit_profiles(
        roots: Vec<PathBuf>,
        quick_limits: StorageAnalysisLimits,
        thorough_limits: StorageAnalysisLimits,
    ) -> Result<Self, String> {
        if roots.is_empty() {
            return Err("at least one managed root is required".to_owned());
        }
        if roots.len() > MAX_CONFIGURED_ROOTS {
            return Err("too many analysis roots are configured".to_owned());
        }
        for limits in [quick_limits, thorough_limits] {
            if limits.max_entries == 0
                || limits.max_results_per_list == 0
                || limits.max_concurrent_jobs == 0
                || limits.max_retained_jobs < limits.max_concurrent_jobs
                || limits.max_response_bytes < 1024
            {
                return Err("storage analysis limits are invalid".to_owned());
            }
        }

        let mut opened = Vec::new();
        for configured in roots {
            let path = normalize_absolute_analysis_path(&configured)?;
            ensure_visible(&path)?;
            let descriptor = open_absolute_directory_without_symlinks(&path).map_err(|_| {
                format!(
                    "managed root {} is unavailable or contains a symbolic link",
                    path.display()
                )
            })?;
            opened.push(StorageAnalysisRoot {
                path,
                descriptor: Arc::new(descriptor),
            });
        }
        opened.sort_by(|left, right| left.path.cmp(&right.path));
        opened.dedup_by(|left, right| left.path == right.path);
        opened.sort_by_key(|root| std::cmp::Reverse(root.path.components().count()));

        Ok(Self {
            roots: Arc::new(opened),
            jobs: Arc::new(StorageAnalysisJobs {
                inner: Mutex::new(StorageAnalysisJobRegistry::default()),
            }),
            quick_limits,
            thorough_limits,
        })
    }

    #[cfg(test)]
    pub fn start(&self, path: &str) -> Result<StorageAnalysisStart, String> {
        self.start_with_mode(path, StorageAnalysisMode::Quick)
    }

    pub fn start_with_mode(
        &self,
        path: &str,
        mode: StorageAnalysisMode,
    ) -> Result<StorageAnalysisStart, String> {
        let limits = match mode {
            StorageAnalysisMode::Quick => self.quick_limits,
            StorageAnalysisMode::Thorough => self.thorough_limits,
        };
        let (descriptor, normalized_path, target_device) = self.open_target(path)?;
        let job_id = Uuid::new_v4();
        let cancellation = Arc::new(AtomicBool::new(false));
        {
            let mut registry = lock_jobs(&self.jobs);
            prune_storage_analysis_jobs(&mut registry, limits.max_retained_jobs);
            if registry.active >= limits.max_concurrent_jobs {
                return Err("too many storage analyses are already running".to_owned());
            }
            registry.active = registry.active.saturating_add(1);
            registry.jobs.insert(
                job_id,
                StorageAnalysisJobRecord {
                    id: job_id,
                    requested_path: normalized_path.clone(),
                    state: StorageAnalysisJobState::Queued,
                    progress: StorageAnalysisProgress::default(),
                    created_unix_ms: now_unix_ms(),
                    started_unix_ms: None,
                    finished_unix_ms: None,
                    cancel_requested: false,
                    cancellation: Arc::clone(&cancellation),
                    result: None,
                    error: None,
                },
            );
        }

        let jobs = Arc::clone(&self.jobs);
        let worker_path = PathBuf::from(&normalized_path);
        let worker = StorageAnalysisWorker {
            jobs: Arc::clone(&jobs),
            job_id,
            descriptor,
            root_path: worker_path,
            target_device,
            cancellation,
            mode,
            limits,
        };
        let spawn = thread::Builder::new()
            .name(format!("helix-storage-{}", job_id.simple()))
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_storage_analysis_job(worker);
                }));
                if outcome.is_err() {
                    fail_storage_analysis_job(&jobs, job_id);
                }
            });
        if spawn.is_err() {
            let mut registry = lock_jobs(&self.jobs);
            registry.active = registry.active.saturating_sub(1);
            if let Some(job) = registry.jobs.get_mut(&job_id) {
                job.state = StorageAnalysisJobState::Failed;
                job.finished_unix_ms = Some(now_unix_ms());
                job.error = Some("the storage analysis worker could not start".to_owned());
            }
            return Err("the storage analysis worker could not start".to_owned());
        }

        Ok(StorageAnalysisStart {
            job_id: job_id.hyphenated().to_string(),
            state: StorageAnalysisJobState::Queued,
        })
    }

    pub fn status(&self, job_id: &str) -> Result<StorageAnalysisStatus, String> {
        let id = parse_analysis_job_id(job_id)?;
        let status = {
            let registry = lock_jobs(&self.jobs);
            let job = registry
                .jobs
                .get(&id)
                .ok_or_else(|| "the storage analysis job does not exist".to_owned())?;
            storage_analysis_status(job)
        };
        if serde_json::to_vec(&status)
            .map_err(|_| "the storage analysis status could not be encoded".to_owned())?
            .len()
            > self
                .quick_limits
                .max_response_bytes
                .max(self.thorough_limits.max_response_bytes)
        {
            return Err("the storage analysis status exceeded its response limit".to_owned());
        }
        Ok(status)
    }

    pub fn cancel(&self, job_id: &str) -> Result<StorageAnalysisStatus, String> {
        let id = parse_analysis_job_id(job_id)?;
        let status = {
            let mut registry = lock_jobs(&self.jobs);
            let job = registry
                .jobs
                .get_mut(&id)
                .ok_or_else(|| "the storage analysis job does not exist".to_owned())?;
            if !job.state.is_terminal() {
                job.cancel_requested = true;
                job.cancellation.store(true, AtomicOrdering::Release);
            }
            storage_analysis_status(job)
        };
        Ok(status)
    }

    fn open_target(&self, path: &str) -> Result<(OwnedFd, String, rustix::fs::Dev), String> {
        if path.is_empty() || path.len() > 4096 || path.contains('\0') {
            return Err("the path is invalid".to_owned());
        }
        let normalized = normalize_absolute_analysis_path(Path::new(path))?;
        ensure_visible(&normalized)?;
        let root = self
            .roots
            .iter()
            .find(|root| normalized.starts_with(&root.path))
            .ok_or_else(|| "the selected path is outside a managed storage root".to_owned())?;
        let relative = normalized
            .strip_prefix(&root.path)
            .map_err(|_| "the selected path is outside a managed storage root".to_owned())?;
        // `dup` would share the directory stream offset with the retained root
        // descriptor. Opening `.` creates a fresh file description so every
        // analysis starts at the beginning of the directory.
        let mut descriptor = open_directory_at(&root.descriptor, c".")
            .map_err(|_| "the storage analysis target is unavailable".to_owned())?;
        for component in relative.components() {
            let std::path::Component::Normal(name) = component else {
                return Err("the path is invalid".to_owned());
            };
            let name =
                CString::new(name.as_bytes()).map_err(|_| "the path is invalid".to_owned())?;
            descriptor = open_directory_at(&descriptor, name.as_c_str()).map_err(|_| {
                "the storage analysis target is unavailable or contains a symbolic link".to_owned()
            })?;
        }
        let stat = rustix::fs::fstat(&descriptor)
            .map_err(|_| "the storage analysis target is unavailable".to_owned())?;
        Ok((
            descriptor,
            normalized.to_string_lossy().into_owned(),
            stat.st_dev,
        ))
    }
}

struct StorageAnalysisWorker {
    jobs: Arc<StorageAnalysisJobs>,
    job_id: Uuid,
    descriptor: OwnedFd,
    root_path: PathBuf,
    target_device: rustix::fs::Dev,
    cancellation: Arc<AtomicBool>,
    mode: StorageAnalysisMode,
    limits: StorageAnalysisLimits,
}

fn run_storage_analysis_job(worker: StorageAnalysisWorker) {
    let StorageAnalysisWorker {
        jobs,
        job_id,
        descriptor,
        root_path,
        target_device,
        cancellation,
        mode,
        limits,
    } = worker;
    let started_unix_ms = now_unix_ms();
    {
        let mut registry = lock_jobs(&jobs);
        if let Some(job) = registry.jobs.get_mut(&job_id) {
            job.state = StorageAnalysisJobState::Running;
            job.started_unix_ms = Some(started_unix_ms);
        }
    }

    let mut scan = StorageScan {
        job_id,
        jobs: Arc::clone(&jobs),
        cancellation,
        started: Instant::now(),
        target_device,
        mode,
        limits,
        progress: StorageAnalysisProgress::default(),
        errors: StorageAnalysisErrors::default(),
        skipped: StorageAnalysisSkipped::default(),
        stop_reason: None,
        files_seen: 0,
        folders_seen: 1,
        largest_files: BinaryHeap::new(),
        recursive_folders: BinaryHeap::new(),
        immediate_folders: BinaryHeap::new(),
        hard_link_inodes: HashSet::new(),
    };
    let root_totals = scan.scan_directory(descriptor, &root_path, 0);
    scan.keep_folder(&root_path, root_totals);
    scan.publish_progress(true);
    let finished_unix_ms = now_unix_ms();
    let cancelled = scan.stop_reason == Some(StorageAnalysisStopReason::Cancelled);
    let mut result = scan.finish(&root_path);
    fit_storage_analysis_response(
        &mut result,
        job_id,
        &root_path,
        started_unix_ms,
        finished_unix_ms,
        cancelled,
        limits.max_response_bytes.saturating_sub(1024),
    );

    let mut registry = lock_jobs(&jobs);
    registry.active = registry.active.saturating_sub(1);
    if let Some(job) = registry.jobs.get_mut(&job_id) {
        job.progress.percent = Some(100);
        job.state = if cancelled {
            StorageAnalysisJobState::Cancelled
        } else {
            StorageAnalysisJobState::Complete
        };
        job.finished_unix_ms = Some(finished_unix_ms);
        job.result = Some(result);
    }
}

fn fail_storage_analysis_job(jobs: &StorageAnalysisJobs, job_id: Uuid) {
    let mut registry = lock_jobs(jobs);
    let was_active = registry
        .jobs
        .get(&job_id)
        .is_some_and(|job| !job.state.is_terminal());
    if was_active {
        registry.active = registry.active.saturating_sub(1);
    }
    if let Some(job) = registry.jobs.get_mut(&job_id) {
        job.state = StorageAnalysisJobState::Failed;
        job.finished_unix_ms = Some(now_unix_ms());
        job.error = Some("the storage analysis worker failed safely".to_owned());
    }
}

impl StorageScan {
    fn scan_directory(&mut self, descriptor: OwnedFd, path: &Path, depth: u16) -> DirectoryTotals {
        self.progress.directories_scanned = self.progress.directories_scanned.saturating_add(1);
        let mut totals = DirectoryTotals {
            immediate_bytes: 0,
            recursive_bytes: 0,
            immediate_allocated_bytes: 0,
            recursive_allocated_bytes: 0,
            immediate_complete: true,
            recursive_complete: true,
        };
        let mut buffer = Vec::<u8>::with_capacity(STORAGE_ANALYSIS_READ_BUFFER_BYTES);
        let mut directory = rustix::fs::RawDir::new(&descriptor, buffer.spare_capacity_mut());

        loop {
            if self.external_limit_reached() {
                totals.immediate_complete = false;
                totals.recursive_complete = false;
                break;
            }
            let entry = match directory.next() {
                None => break,
                Some(Ok(entry)) => entry,
                Some(Err(error)) => {
                    self.record_filesystem_error(error);
                    totals.immediate_complete = false;
                    totals.recursive_complete = false;
                    break;
                }
            };
            let raw_name = entry.file_name();
            if raw_name.to_bytes() == b"." || raw_name.to_bytes() == b".." {
                continue;
            }
            if self.progress.entries_scanned >= self.limits.max_entries {
                self.stop_reason = Some(StorageAnalysisStopReason::EntryLimit);
                totals.immediate_complete = false;
                totals.recursive_complete = false;
                break;
            }
            self.progress.entries_scanned = self.progress.entries_scanned.saturating_add(1);
            if !self.limits.per_entry_delay.is_zero() {
                thread::sleep(self.limits.per_entry_delay);
            }
            let child_path = path.join(OsStr::from_bytes(raw_name.to_bytes()));
            if is_blocked(&child_path) {
                self.skipped.restricted_paths = self.skipped.restricted_paths.saturating_add(1);
                self.publish_progress(false);
                continue;
            }

            let metadata = match rustix::fs::statat(
                &descriptor,
                raw_name,
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(metadata) => metadata,
                Err(error) => {
                    self.record_filesystem_error(error);
                    totals.immediate_complete = false;
                    totals.recursive_complete = false;
                    self.publish_progress(false);
                    continue;
                }
            };
            let kind = rustix::fs::FileType::from_raw_mode(metadata.st_mode);
            if kind.is_symlink() {
                self.skipped.symbolic_links = self.skipped.symbolic_links.saturating_add(1);
                self.publish_progress(false);
                continue;
            }
            if metadata.st_dev != self.target_device {
                self.skipped.other_filesystems = self.skipped.other_filesystems.saturating_add(1);
                self.publish_progress(false);
                continue;
            }
            if kind.is_file() {
                self.files_seen = self.files_seen.saturating_add(1);
                self.progress.files_scanned = self.progress.files_scanned.saturating_add(1);
                if metadata.st_nlink > 1
                    && !self
                        .hard_link_inodes
                        .insert((metadata.st_dev, metadata.st_ino))
                {
                    self.skipped.hard_link_aliases =
                        self.skipped.hard_link_aliases.saturating_add(1);
                    self.publish_progress(false);
                    continue;
                }
                let Ok(bytes) = u64::try_from(metadata.st_size) else {
                    self.record_size_overflow();
                    totals.immediate_complete = false;
                    totals.recursive_complete = false;
                    self.publish_progress(false);
                    continue;
                };
                let Some(allocated_bytes) = u64::try_from(metadata.st_blocks)
                    .ok()
                    .and_then(|blocks| blocks.checked_mul(512))
                else {
                    self.record_size_overflow();
                    totals.immediate_complete = false;
                    totals.recursive_complete = false;
                    self.publish_progress(false);
                    continue;
                };
                let scanned_bytes = self.progress.bytes_scanned;
                self.progress.bytes_scanned =
                    self.add_bytes(scanned_bytes, bytes, &mut totals.immediate_complete);
                let scanned_allocated_bytes = self.progress.allocated_bytes_scanned;
                self.progress.allocated_bytes_scanned = self.add_bytes(
                    scanned_allocated_bytes,
                    allocated_bytes,
                    &mut totals.immediate_complete,
                );
                totals.immediate_bytes = self.add_bytes(
                    totals.immediate_bytes,
                    bytes,
                    &mut totals.immediate_complete,
                );
                totals.recursive_bytes = self.add_bytes(
                    totals.recursive_bytes,
                    bytes,
                    &mut totals.recursive_complete,
                );
                totals.immediate_allocated_bytes = self.add_bytes(
                    totals.immediate_allocated_bytes,
                    allocated_bytes,
                    &mut totals.immediate_complete,
                );
                totals.recursive_allocated_bytes = self.add_bytes(
                    totals.recursive_allocated_bytes,
                    allocated_bytes,
                    &mut totals.recursive_complete,
                );
                if let Some((path, name)) = representable_analysis_path(&child_path) {
                    self.keep_file(StorageAnalysisFile {
                        path,
                        name,
                        kind: StorageAnalysisEntryKind::File,
                        bytes,
                        allocated_bytes,
                    });
                } else {
                    self.skipped.unrepresentable_names =
                        self.skipped.unrepresentable_names.saturating_add(1);
                }
            } else if kind.is_dir() {
                self.folders_seen = self.folders_seen.saturating_add(1);
                if depth >= self.limits.max_depth {
                    self.skipped.depth_limited_directories =
                        self.skipped.depth_limited_directories.saturating_add(1);
                    totals.recursive_complete = false;
                    self.publish_progress(false);
                    continue;
                }
                let child_descriptor = match open_directory_at(&descriptor, raw_name) {
                    Ok(descriptor) => descriptor,
                    Err(error) => {
                        self.record_filesystem_error(error);
                        totals.recursive_complete = false;
                        self.publish_progress(false);
                        continue;
                    }
                };
                let child = self.scan_directory(child_descriptor, &child_path, depth + 1);
                totals.recursive_bytes = self.add_bytes(
                    totals.recursive_bytes,
                    child.recursive_bytes,
                    &mut totals.recursive_complete,
                );
                totals.recursive_allocated_bytes = self.add_bytes(
                    totals.recursive_allocated_bytes,
                    child.recursive_allocated_bytes,
                    &mut totals.recursive_complete,
                );
                totals.recursive_complete &= child.recursive_complete;
                self.keep_folder(&child_path, child);
                if self.stop_reason.is_some() {
                    totals.immediate_complete = false;
                    totals.recursive_complete = false;
                    break;
                }
            } else {
                self.skipped.special_files = self.skipped.special_files.saturating_add(1);
            }
            self.publish_progress(false);
        }
        totals
    }

    fn external_limit_reached(&mut self) -> bool {
        if self.stop_reason.is_some() {
            return true;
        }
        if self.cancellation.load(AtomicOrdering::Acquire) {
            self.stop_reason = Some(StorageAnalysisStopReason::Cancelled);
            return true;
        }
        if self.started.elapsed() >= self.limits.max_duration {
            self.stop_reason = Some(StorageAnalysisStopReason::DurationLimit);
            return true;
        }
        false
    }

    fn add_bytes(&mut self, left: u64, right: u64, complete: &mut bool) -> u64 {
        match left.checked_add(right) {
            Some(total) => total,
            None => {
                self.record_size_overflow();
                *complete = false;
                u64::MAX
            }
        }
    }

    fn record_size_overflow(&mut self) {
        self.errors.total = self.errors.total.saturating_add(1);
        self.errors.size_overflows = self.errors.size_overflows.saturating_add(1);
    }

    fn record_filesystem_error(&mut self, error: rustix::io::Errno) {
        self.errors.total = self.errors.total.saturating_add(1);
        match error.kind() {
            io::ErrorKind::PermissionDenied => {
                self.errors.permission_denied = self.errors.permission_denied.saturating_add(1);
            }
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => {
                self.errors.filesystem_races = self.errors.filesystem_races.saturating_add(1);
            }
            _ => {
                self.errors.metadata_failures = self.errors.metadata_failures.saturating_add(1);
            }
        }
    }

    fn keep_file(&mut self, file: StorageAnalysisFile) {
        self.largest_files.push(Reverse(RankedFile(file)));
        if self.largest_files.len() > self.limits.max_results_per_list {
            self.largest_files.pop();
        }
    }

    fn keep_folder(&mut self, path: &Path, totals: DirectoryTotals) {
        let Some((path, name)) = representable_analysis_path(path) else {
            self.skipped.unrepresentable_names =
                self.skipped.unrepresentable_names.saturating_add(1);
            return;
        };
        let folder = StorageAnalysisFolder {
            path,
            name,
            kind: StorageAnalysisEntryKind::Directory,
            immediate_bytes: totals.immediate_bytes,
            recursive_bytes: totals.recursive_bytes,
            immediate_allocated_bytes: totals.immediate_allocated_bytes,
            recursive_allocated_bytes: totals.recursive_allocated_bytes,
            immediate_complete: totals.immediate_complete,
            recursive_complete: totals.recursive_complete,
        };
        self.recursive_folders.push(Reverse(RankedFolder {
            bytes: folder.recursive_allocated_bytes,
            folder: folder.clone(),
        }));
        if self.recursive_folders.len() > self.limits.max_results_per_list {
            self.recursive_folders.pop();
        }
        self.immediate_folders.push(Reverse(RankedFolder {
            bytes: folder.immediate_allocated_bytes,
            folder,
        }));
        if self.immediate_folders.len() > self.limits.max_results_per_list {
            self.immediate_folders.pop();
        }
    }

    fn publish_progress(&self, force: bool) {
        let interval = self.limits.progress_interval.max(1);
        if !force && self.progress.entries_scanned.checked_rem(interval) != Some(0) {
            return;
        }
        let entry_percent = self
            .progress
            .entries_scanned
            .saturating_mul(100)
            .checked_div(self.limits.max_entries)
            .unwrap_or(0);
        let elapsed_percent = u64::try_from(self.started.elapsed().as_millis())
            .unwrap_or(u64::MAX)
            .saturating_mul(100)
            .checked_div(
                u64::try_from(self.limits.max_duration.as_millis())
                    .unwrap_or(u64::MAX)
                    .max(1),
            )
            .unwrap_or(0);
        let mut progress = self.progress.clone();
        progress.percent = None;
        progress.entry_budget_percent = u8::try_from(entry_percent.min(100)).unwrap_or(100);
        progress.duration_budget_percent = u8::try_from(elapsed_percent.min(100)).unwrap_or(100);
        let mut registry = lock_jobs(&self.jobs);
        if let Some(job) = registry.jobs.get_mut(&self.job_id) {
            job.progress = progress;
        }
    }

    fn finish(self, root_path: &Path) -> StorageAnalysisResult {
        let mut largest_files = self
            .largest_files
            .into_iter()
            .map(|Reverse(item)| item.0)
            .collect::<Vec<_>>();
        largest_files.sort_by(|left, right| {
            right
                .allocated_bytes
                .cmp(&left.allocated_bytes)
                .then_with(|| right.bytes.cmp(&left.bytes))
                .then_with(|| left.path.cmp(&right.path))
        });
        let mut recursive_folders = self
            .recursive_folders
            .into_iter()
            .map(|Reverse(item)| item.folder)
            .collect::<Vec<_>>();
        recursive_folders.sort_by(|left, right| {
            right
                .recursive_allocated_bytes
                .cmp(&left.recursive_allocated_bytes)
                .then_with(|| right.recursive_bytes.cmp(&left.recursive_bytes))
                .then_with(|| left.path.cmp(&right.path))
        });
        let mut immediate_folders = self
            .immediate_folders
            .into_iter()
            .map(|Reverse(item)| item.folder)
            .collect::<Vec<_>>();
        immediate_folders.sort_by(|left, right| {
            right
                .immediate_allocated_bytes
                .cmp(&left.immediate_allocated_bytes)
                .then_with(|| right.immediate_bytes.cmp(&left.immediate_bytes))
                .then_with(|| left.path.cmp(&right.path))
        });
        let file_results_omitted = self
            .files_seen
            .saturating_sub(u64::try_from(largest_files.len()).unwrap_or(u64::MAX));
        let recursive_folder_results_omitted = self
            .folders_seen
            .saturating_sub(u64::try_from(recursive_folders.len()).unwrap_or(u64::MAX));
        let immediate_folder_results_omitted = self
            .folders_seen
            .saturating_sub(u64::try_from(immediate_folders.len()).unwrap_or(u64::MAX));
        let max_duration_ms =
            u64::try_from(self.limits.max_duration.as_millis()).unwrap_or(u64::MAX);
        let coverage_incomplete = self.stop_reason.is_some()
            || self.errors.total > 0
            || self.skipped.depth_limited_directories > 0;
        StorageAnalysisResult {
            root_path: root_path.to_string_lossy().into_owned(),
            apparent_bytes_scanned: self.progress.bytes_scanned,
            allocated_bytes_scanned: self.progress.allocated_bytes_scanned,
            truncated: coverage_incomplete,
            stop_reason: self.stop_reason,
            errors: self.errors,
            skipped: self.skipped,
            file_results_omitted,
            recursive_folder_results_omitted,
            immediate_folder_results_omitted,
            response_results_omitted: 0,
            largest_files,
            largest_folders_by_recursive_bytes: recursive_folders,
            largest_folders_by_immediate_bytes: immediate_folders,
            limits: StorageAnalysisAppliedLimits {
                mode: self.mode,
                max_duration_ms,
                max_entries: self.limits.max_entries,
                max_depth: self.limits.max_depth,
                max_results_per_list: self.limits.max_results_per_list,
                max_response_bytes: self.limits.max_response_bytes,
                stay_on_target_filesystem: true,
                follows_symbolic_links: false,
            },
        }
    }
}

fn fit_storage_analysis_response(
    result: &mut StorageAnalysisResult,
    job_id: Uuid,
    root_path: &Path,
    started_unix_ms: u64,
    finished_unix_ms: u64,
    cancelled: bool,
    max_response_bytes: usize,
) {
    loop {
        let status = StorageAnalysisStatus {
            job_id: job_id.hyphenated().to_string(),
            requested_path: root_path.to_string_lossy().into_owned(),
            state: if cancelled {
                StorageAnalysisJobState::Cancelled
            } else {
                StorageAnalysisJobState::Complete
            },
            progress: StorageAnalysisProgress {
                percent: Some(100),
                entry_budget_percent: 100,
                duration_budget_percent: 100,
                entries_scanned: 0,
                files_scanned: 0,
                directories_scanned: 0,
                bytes_scanned: result.apparent_bytes_scanned,
                allocated_bytes_scanned: result.allocated_bytes_scanned,
            },
            created_unix_ms: started_unix_ms,
            started_unix_ms: Some(started_unix_ms),
            finished_unix_ms: Some(finished_unix_ms),
            cancel_requested: cancelled,
            result: Some(result.clone()),
            error: None,
        };
        let encoded_bytes = serde_json::to_vec(&status)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX);
        if encoded_bytes <= max_response_bytes {
            break;
        }
        let remaining = result
            .largest_files
            .len()
            .saturating_add(result.largest_folders_by_recursive_bytes.len())
            .saturating_add(result.largest_folders_by_immediate_bytes.len());
        if remaining == 0 {
            break;
        }
        let excess = encoded_bytes.saturating_sub(max_response_bytes);
        let removal_count = excess
            .saturating_mul(remaining)
            .checked_div(encoded_bytes.max(1))
            .unwrap_or(remaining)
            .saturating_add(1)
            .min(remaining);
        for _ in 0..removal_count {
            remove_smallest_storage_result(result);
        }
        result.response_results_omitted = result
            .response_results_omitted
            .saturating_add(u64::try_from(removal_count).unwrap_or(u64::MAX));
    }
}

fn remove_smallest_storage_result(result: &mut StorageAnalysisResult) {
    if result.largest_folders_by_recursive_bytes.len()
        >= result.largest_folders_by_immediate_bytes.len()
        && result.largest_folders_by_recursive_bytes.len() >= result.largest_files.len()
    {
        result.largest_folders_by_recursive_bytes.pop();
    } else if result.largest_folders_by_immediate_bytes.len() >= result.largest_files.len() {
        result.largest_folders_by_immediate_bytes.pop();
    } else {
        result.largest_files.pop();
    }
}

fn storage_analysis_status(job: &StorageAnalysisJobRecord) -> StorageAnalysisStatus {
    StorageAnalysisStatus {
        job_id: job.id.hyphenated().to_string(),
        requested_path: job.requested_path.clone(),
        state: job.state,
        progress: job.progress.clone(),
        created_unix_ms: job.created_unix_ms,
        started_unix_ms: job.started_unix_ms,
        finished_unix_ms: job.finished_unix_ms,
        cancel_requested: job.cancel_requested,
        result: job.result.clone(),
        error: job.error.clone(),
    }
}

fn prune_storage_analysis_jobs(registry: &mut StorageAnalysisJobRegistry, max_retained: usize) {
    while registry.jobs.len() >= max_retained {
        let oldest = registry
            .jobs
            .iter()
            .filter(|(_, job)| job.state.is_terminal())
            .min_by_key(|(_, job)| job.finished_unix_ms.unwrap_or(u64::MAX))
            .map(|(id, _)| *id);
        let Some(oldest) = oldest else {
            break;
        };
        registry.jobs.remove(&oldest);
    }
}

fn lock_jobs(jobs: &StorageAnalysisJobs) -> std::sync::MutexGuard<'_, StorageAnalysisJobRegistry> {
    jobs.inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn parse_analysis_job_id(job_id: &str) -> Result<Uuid, String> {
    if job_id.len() > 64 || job_id.chars().any(char::is_control) {
        return Err("the storage analysis job id is invalid".to_owned());
    }
    let id =
        Uuid::parse_str(job_id).map_err(|_| "the storage analysis job id is invalid".to_owned())?;
    if id.hyphenated().to_string() != job_id {
        return Err("the storage analysis job id is invalid".to_owned());
    }
    Ok(id)
}

fn normalize_absolute_analysis_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("the path must be absolute".to_owned());
    }
    let text = path
        .to_str()
        .ok_or_else(|| "the path is invalid".to_owned())?;
    if text.is_empty() || text.len() > 4096 || text.chars().any(char::is_control) {
        return Err("the path is invalid".to_owned());
    }
    let mut normalized = PathBuf::from("/");
    for component in path.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                if name.as_bytes().is_empty() || name.as_bytes().contains(&0) {
                    return Err("the path is invalid".to_owned());
                }
                normalized.push(name);
            }
            _ => return Err("the path cannot contain traversal components".to_owned()),
        }
    }
    Ok(normalized)
}

fn open_absolute_directory_without_symlinks(path: &Path) -> rustix::io::Result<OwnedFd> {
    let mut descriptor = rustix::fs::open("/", directory_open_flags(), rustix::fs::Mode::empty())?;
    for component in path.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                let name = CString::new(name.as_bytes()).map_err(|_| rustix::io::Errno::INVAL)?;
                descriptor = open_directory_at(&descriptor, name.as_c_str())?;
            }
            _ => return Err(rustix::io::Errno::INVAL),
        }
    }
    Ok(descriptor)
}

fn open_directory_at<Fd: std::os::fd::AsFd>(
    descriptor: Fd,
    name: &CStr,
) -> rustix::io::Result<OwnedFd> {
    rustix::fs::openat(
        descriptor,
        name,
        directory_open_flags(),
        rustix::fs::Mode::empty(),
    )
}

fn directory_open_flags() -> rustix::fs::OFlags {
    rustix::fs::OFlags::RDONLY
        | rustix::fs::OFlags::DIRECTORY
        | rustix::fs::OFlags::CLOEXEC
        | rustix::fs::OFlags::NOFOLLOW
}

fn representable_analysis_path(path: &Path) -> Option<(String, String)> {
    let path = path.to_str()?;
    if path.len() > 4096 || path.chars().any(char::is_control) {
        return None;
    }
    let name = Path::new(path)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("/");
    if name.len() > 255 || name.chars().any(char::is_control) {
        return None;
    }
    Some((path.to_owned(), name.to_owned()))
}

fn serialize_u64_decimal<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&value.to_string())
}

fn canonical_existing(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.len() > 4096 || path.contains('\0') {
        return Err("the path is invalid".to_owned());
    }
    let input = Path::new(path);
    if !input.is_absolute() {
        return Err("the path must be absolute".to_owned());
    }
    fs::canonicalize(input).map_err(file_error)
}

fn ensure_visible(path: &Path) -> Result<(), String> {
    if is_blocked(path) {
        Err("this system path is intentionally unavailable in the file manager".to_owned())
    } else {
        Ok(())
    }
}

fn is_blocked(path: &Path) -> bool {
    BLOCKED_ROOTS
        .iter()
        .map(Path::new)
        .any(|root| path.starts_with(root))
        || BLOCKED_FILES
            .iter()
            .map(Path::new)
            .any(|root| path.starts_with(root))
}

pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 255
        || name.contains(['/', '\\', '\0'])
        || name.chars().any(char::is_control)
    {
        return Err("the name is invalid".to_owned());
    }
    Ok(())
}

fn ensure_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err("an item with that name already exists".to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(file_error(error)),
    }
}

fn inherit_owner(parent: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::metadata(parent).map_err(file_error)?;
    chown(target, Some(metadata.uid()), Some(metadata.gid())).map_err(file_error)
}

fn inherit_owner_and_directory_mode(parent: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::metadata(parent).map_err(file_error)?;
    fs::set_permissions(
        target,
        fs::Permissions::from_mode(metadata.permissions().mode() & 0o777),
    )
    .map_err(file_error)?;
    chown(target, Some(metadata.uid()), Some(metadata.gid())).map_err(file_error)
}

fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(file_error)
}

fn modified_unix_ms(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis()
        .try_into()
        .ok()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn directory_name_key(name: &str) -> (String, String) {
    (name.to_lowercase(), name.to_owned())
}

fn file_error(error: io::Error) -> String {
    match error.kind() {
        io::ErrorKind::NotFound => "the selected path does not exist".to_owned(),
        io::ErrorKind::PermissionDenied => {
            "the operating system denied access to that path".to_owned()
        }
        io::ErrorKind::AlreadyExists => "an item with that name already exists".to_owned(),
        io::ErrorKind::StorageFull => "the destination filesystem is full".to_owned(),
        _ => "the filesystem operation failed".to_owned(),
    }
}

pub(crate) fn decode_upload_chunk(data_base64: &str) -> Result<Vec<u8>, String> {
    if data_base64.is_empty() || data_base64.len() > MAX_FILE_UPLOAD_CHUNK_BYTES * 2 {
        return Err("the upload chunk is invalid".to_owned());
    }
    let data = STANDARD
        .decode(data_base64)
        .map_err(|_| "the upload chunk encoding is invalid".to_owned())?;
    if data.is_empty() || data.len() > MAX_FILE_UPLOAD_CHUNK_BYTES {
        return Err("the upload chunk is outside the size limit".to_owned());
    }
    Ok(data)
}

pub(crate) fn prune_uploads(uploads: &mut HashMap<String, StreamingUpload>) {
    uploads.retain(|_, upload| !upload.is_stale());
}

pub(crate) fn upload_lock_error() -> String {
    "upload state is temporarily unavailable".to_owned()
}

pub(crate) fn commit_upload(upload: StreamingUpload) -> Result<PathBuf, String> {
    let parent = upload.parent.clone();
    let destination = upload.finish()?;
    inherit_owner(&parent, &destination)?;
    sync_directory(&parent)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn file_manager_rejects_too_many_roots() {
        let roots = (0..=MAX_CONFIGURED_ROOTS)
            .map(|index| PathBuf::from(format!("/srv/helix/root-{index}")))
            .collect();
        let error = FileManager::new(roots).err().expect("root cap");
        assert!(error.contains("too many managed roots"));
    }

    #[test]
    fn storage_analysis_rejects_too_many_roots() {
        let roots = (0..=MAX_CONFIGURED_ROOTS)
            .map(|index| PathBuf::from(format!("/srv/helix/analysis-{index}")))
            .collect();
        let error = StorageAnalysisManager::new(roots).err().expect("root cap");
        assert!(error.contains("too many analysis roots"));
    }

    #[test]
    fn storage_analysis_charges_hard_linked_data_once() {
        let temporary = tempfile::tempdir().unwrap();
        let managed = temporary.path().join("managed");
        let first = managed.join("first");
        let second = managed.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let original = first.join("shared.bin");
        fs::write(&original, vec![0x5a; 16 * 1024]).unwrap();
        fs::hard_link(&original, second.join("shared.bin")).unwrap();
        let allocated = fs::metadata(&original)
            .unwrap()
            .blocks()
            .saturating_mul(512);
        let manager = StorageAnalysisManager::with_limits(vec![managed.clone()], analysis_limits())
            .expect("analysis manager");

        let started = manager
            .start(&managed.to_string_lossy())
            .expect("start analysis");
        let status = wait_for_analysis(&manager, &started.job_id);
        let result = status.result.expect("analysis result");

        assert_eq!(result.allocated_bytes_scanned, allocated);
        assert_eq!(result.skipped.hard_link_aliases, 1);
        assert_eq!(result.largest_files.len(), 1);
    }

    fn analysis_limits() -> StorageAnalysisLimits {
        StorageAnalysisLimits {
            max_duration: Duration::from_secs(2),
            max_entries: 10_000,
            max_depth: 16,
            max_results_per_list: 16,
            max_concurrent_jobs: 2,
            max_retained_jobs: 8,
            max_response_bytes: 64 * 1024,
            progress_interval: 1,
            per_entry_delay: Duration::ZERO,
        }
    }

    fn wait_for_analysis(manager: &StorageAnalysisManager, job_id: &str) -> StorageAnalysisStatus {
        for _ in 0..400 {
            let status = manager.status(job_id).expect("analysis status");
            if status.state.is_terminal() {
                return status;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("storage analysis did not finish in time");
    }

    #[test]
    fn names_reject_traversal_and_separators() {
        for name in ["", ".", "..", "a/b", "a\\b", "bad\0name"] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        assert!(validate_name("Minecraft Worlds").is_ok());
        for path in [
            "/proc/1/status",
            "/root/.ssh/authorized_keys",
            "/etc/shadow",
        ] {
            assert!(is_blocked(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn file_upload_writes_bounded_regular_files_and_rejects_collisions() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        fs::create_dir(&managed).expect("managed root");
        let manager = FileManager::new(vec![managed.clone()]).expect("file manager");
        let parent = managed.to_str().unwrap();
        let payload = b"plugin-bytes";
        let start = manager
            .begin_upload(parent, "WorldGuard.jar", payload.len() as u64)
            .expect("begin");
        assert_eq!(start.max_chunk_bytes, MAX_FILE_UPLOAD_CHUNK_BYTES as u64);
        manager
            .write_upload_chunk(
                &start.upload_id,
                FileUploadPurpose::Storage,
                0,
                &STANDARD.encode(payload),
            )
            .expect("chunk");
        let finished = manager
            .finish_upload(&start.upload_id, FileUploadPurpose::Storage)
            .expect("finish");
        let written = managed.join("WorldGuard.jar");
        assert_eq!(finished.path, written.to_string_lossy());
        assert_eq!(fs::read(&written).unwrap(), payload);

        assert!(manager.begin_upload(parent, "WorldGuard.jar", 4).is_err());
        assert!(manager.begin_upload(parent, "../escape.jar", 4).is_err());
        assert!(manager.begin_upload(parent, "empty.bin", 0).is_err());
        let too_large = manager.begin_upload(parent, "huge.bin", MAX_STORAGE_UPLOAD_BYTES + 1);
        assert!(too_large.is_err());
    }

    #[test]
    fn file_upload_rejects_out_of_order_chunks_and_wrong_purpose() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        fs::create_dir(&managed).expect("managed root");
        let manager = FileManager::new(vec![managed.clone()]).expect("file manager");
        let start = manager
            .begin_upload(managed.to_str().unwrap(), "data.bin", 4)
            .expect("begin");
        assert!(
            manager
                .write_upload_chunk(
                    &start.upload_id,
                    FileUploadPurpose::CustomJar,
                    0,
                    &STANDARD.encode(b"data"),
                )
                .is_err()
        );
        assert!(
            manager
                .write_upload_chunk(
                    &start.upload_id,
                    FileUploadPurpose::Storage,
                    2,
                    &STANDARD.encode(b"da"),
                )
                .is_err()
        );
        manager
            .abort_upload(&start.upload_id, FileUploadPurpose::Storage)
            .expect("abort");
        assert!(!managed.join("data.bin").exists());

        let long = manager
            .begin_upload(managed.to_str().unwrap(), "long.bin", 4)
            .expect("begin long");
        {
            let mut uploads = manager.uploads.lock().expect("upload lock");
            let upload = uploads.get_mut(&long.upload_id).expect("active upload");
            upload.touched = Instant::now() - FILE_UPLOAD_IDLE - Duration::from_secs(1);
            assert!(upload.is_stale());
        }
        manager
            .write_upload_chunk(
                &long.upload_id,
                FileUploadPurpose::Storage,
                0,
                &STANDARD.encode(b"data"),
            )
            .expect("chunk after idle mark");
        {
            let mut uploads = manager.uploads.lock().expect("upload lock");
            prune_uploads(&mut uploads);
            let upload = uploads.get(&long.upload_id).expect("upload still active");
            assert!(!upload.is_stale());
        }
        manager
            .abort_upload(&long.upload_id, FileUploadPurpose::Storage)
            .expect("abort long");
        assert!(
            fs::read_dir(&managed)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".helix-upload-"))
        );
    }

    #[test]
    fn managed_root_is_required_for_mutation() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        let outside = temporary.path().join("outside");
        fs::create_dir(&managed).expect("managed root");
        fs::create_dir(&outside).expect("outside root");
        let manager = FileManager::new(vec![managed.clone()]).expect("file manager");
        assert!(
            manager
                .create_directory(managed.to_str().unwrap(), "created")
                .is_ok()
        );
        assert!(
            manager
                .create_directory(outside.to_str().unwrap(), "blocked")
                .is_err()
        );
    }

    #[test]
    fn directory_listing_pages_every_representable_name_without_duplicates() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        fs::create_dir(&managed).expect("managed root");
        for index in 0..61 {
            fs::write(
                managed.join(format!("entry-{index:03}.txt")),
                index.to_string(),
            )
            .expect("fixture file");
        }
        let manager = FileManager::new(vec![managed.clone()]).expect("file manager");
        let root = managed.to_str().expect("UTF-8 root");

        let first = manager.list(root, None, 25).expect("first page");
        assert_eq!(first.total_entries, 61);
        assert_eq!(first.entries.len(), 25);
        assert!(first.has_more);
        assert_eq!(first.page_limit, 25);
        let second = manager
            .list(root, first.next_cursor.as_deref(), 25)
            .expect("second page");
        let third = manager
            .list(root, second.next_cursor.as_deref(), 25)
            .expect("third page");

        assert_eq!(second.entries.len(), 25);
        assert!(second.has_more);
        assert_eq!(third.entries.len(), 11);
        assert!(!third.has_more);
        assert!(third.next_cursor.is_none());
        let names = first
            .entries
            .iter()
            .chain(&second.entries)
            .chain(&third.entries)
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 61);
        assert_eq!(
            names
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            61
        );
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(manager.list(root, None, 24).is_err());
        assert!(manager.list(root, None, 201).is_err());
    }

    #[test]
    fn trash_is_recoverable_and_stays_inside_managed_root() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        fs::create_dir(&managed).expect("managed root");
        fs::write(managed.join("notes.txt"), "hello").expect("fixture file");
        let manager = FileManager::new(vec![managed.clone()]).expect("file manager");
        let result = manager
            .trash(managed.join("notes.txt").to_str().unwrap())
            .expect("trash file");
        assert!(!managed.join("notes.txt").exists());
        assert!(Path::new(&result.recovery_path).starts_with(managed.join(".helix-trash")));
        assert!(Path::new(&result.recovery_path).is_file());
    }

    #[test]
    fn analysis_reports_sorted_file_and_folder_totals_without_following_symlinks() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        let target = managed.join("target");
        let folder = target.join("folder");
        let nested = folder.join("nested");
        let outside = temporary.path().join("outside.bin");
        fs::create_dir_all(&nested).expect("analysis directories");
        fs::write(target.join("largest.bin"), vec![0_u8; 100]).expect("largest file");
        fs::write(folder.join("direct.bin"), vec![0_u8; 20]).expect("direct file");
        fs::write(nested.join("deep.bin"), vec![0_u8; 30]).expect("deep file");
        fs::write(&outside, vec![0_u8; 10_000]).expect("outside file");
        symlink(&outside, target.join("outside-link")).expect("outside symlink");

        let manager = StorageAnalysisManager::with_limits(vec![managed.clone()], analysis_limits())
            .expect("analysis manager");
        let start = manager
            .start(target.to_str().expect("UTF-8 target"))
            .expect("start analysis");
        let status = wait_for_analysis(&manager, &start.job_id);
        assert_eq!(status.state, StorageAnalysisJobState::Complete);
        let result = status.result.expect("analysis result");
        assert_eq!(result.apparent_bytes_scanned, 150);
        assert!(result.allocated_bytes_scanned >= 150);
        assert_eq!(result.errors.total, 0);
        assert_eq!(result.skipped.symbolic_links, 1);
        assert_eq!(result.largest_files[0].name, "largest.bin");
        assert_eq!(result.largest_files[0].bytes, 100);
        assert!(result.largest_files[0].allocated_bytes >= 100);
        let encoded = serde_json::to_value(&result).expect("serialize exact byte fields");
        assert_eq!(encoded["apparent_bytes_scanned"], "150");
        assert!(encoded["allocated_bytes_scanned"].is_string());
        assert_eq!(encoded["largest_files"][0]["bytes"], "100");
        assert!(encoded["largest_files"][0]["allocated_bytes"].is_string());
        assert_eq!(encoded["largest_files"][0]["type"], "file");
        assert!(encoded["largest_files"][0].get("kind").is_none());
        assert!(
            result
                .largest_files
                .iter()
                .all(|file| file.path.as_str() != outside.to_string_lossy().as_ref())
        );
        let folder_total = result
            .largest_folders_by_recursive_bytes
            .iter()
            .find(|item| item.path.as_str() == folder.to_string_lossy().as_ref())
            .expect("folder result");
        assert_eq!(folder_total.immediate_bytes, 20);
        assert_eq!(folder_total.recursive_bytes, 50);
        assert!(folder_total.immediate_allocated_bytes >= 20);
        assert!(folder_total.recursive_allocated_bytes >= 50);
        assert!(folder_total.immediate_complete);
        assert!(folder_total.recursive_complete);
        assert!(
            result
                .largest_files
                .windows(2)
                .all(|files| files[0].allocated_bytes >= files[1].allocated_bytes)
        );
    }

    #[test]
    fn analysis_ranks_actual_disk_usage_ahead_of_sparse_logical_length() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        fs::create_dir(&managed).expect("managed directory");
        let sparse_path = managed.join("large-sparse.bin");
        File::create(&sparse_path)
            .expect("sparse fixture")
            .set_len(64 * 1024 * 1024)
            .expect("sparse fixture length");
        fs::write(managed.join("smaller-dense.bin"), vec![1_u8; 1024 * 1024])
            .expect("dense fixture");

        let manager = StorageAnalysisManager::with_limits(vec![managed.clone()], analysis_limits())
            .expect("analysis manager");
        let result = wait_for_analysis(
            &manager,
            &manager.start(managed.to_str().unwrap()).unwrap().job_id,
        )
        .result
        .expect("analysis result");
        let sparse = result
            .largest_files
            .iter()
            .find(|file| file.name == "large-sparse.bin")
            .expect("sparse result");
        let dense = result
            .largest_files
            .iter()
            .find(|file| file.name == "smaller-dense.bin")
            .expect("dense result");
        assert!(sparse.bytes > dense.bytes);
        assert!(sparse.allocated_bytes < dense.allocated_bytes);
        assert_eq!(result.largest_files[0].name, "smaller-dense.bin");
    }

    #[test]
    fn repeated_analysis_starts_with_a_fresh_directory_cursor() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        fs::create_dir(&managed).expect("managed directory");
        fs::write(managed.join("one.bin"), vec![1_u8; 2048]).expect("first fixture");
        fs::write(managed.join("two.bin"), vec![2_u8; 4096]).expect("second fixture");

        let manager = StorageAnalysisManager::with_limits(vec![managed.clone()], analysis_limits())
            .expect("analysis manager");
        for _ in 0..2 {
            let start = manager
                .start(managed.to_str().expect("UTF-8 target"))
                .expect("start analysis");
            let status = wait_for_analysis(&manager, &start.job_id);
            assert_eq!(status.state, StorageAnalysisJobState::Complete);
            let result = status.result.expect("analysis result");
            assert_eq!(status.progress.entries_scanned, 2);
            assert_eq!(result.apparent_bytes_scanned, 6144);
            assert_eq!(result.largest_files.len(), 2);
        }
    }

    #[test]
    fn analysis_rejects_root_and_target_symlinks_and_parent_traversal() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let real_root = temporary.path().join("real");
        let linked_root = temporary.path().join("linked");
        let outside = temporary.path().join("outside");
        fs::create_dir(&real_root).expect("real root");
        fs::create_dir(&outside).expect("outside directory");
        symlink(&real_root, &linked_root).expect("root symlink");
        assert!(StorageAnalysisManager::with_limits(vec![linked_root], analysis_limits()).is_err());

        symlink(&outside, real_root.join("target-link")).expect("target symlink");
        let manager =
            StorageAnalysisManager::with_limits(vec![real_root.clone()], analysis_limits())
                .expect("analysis manager");
        assert!(
            manager
                .start(real_root.join("target-link").to_str().unwrap())
                .is_err()
        );
        assert!(
            manager
                .start(real_root.join("..").join("outside").to_str().unwrap())
                .is_err()
        );
    }

    #[test]
    fn analysis_marks_depth_and_result_bounds_as_partial() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        let child = managed.join("child");
        fs::create_dir_all(&child).expect("managed directories");
        for (name, bytes) in [("a.bin", 10), ("b.bin", 20), ("c.bin", 30)] {
            fs::write(managed.join(name), vec![0_u8; bytes]).expect("fixture file");
        }
        fs::write(child.join("hidden-by-depth.bin"), vec![0_u8; 1_000]).expect("depth fixture");
        let mut limits = analysis_limits();
        limits.max_depth = 0;
        limits.max_results_per_list = 2;
        let manager = StorageAnalysisManager::with_limits(vec![managed.clone()], limits)
            .expect("analysis manager");
        let start = manager.start(managed.to_str().unwrap()).expect("analysis");
        let result = wait_for_analysis(&manager, &start.job_id)
            .result
            .expect("result");
        assert!(result.truncated);
        assert_eq!(result.skipped.depth_limited_directories, 1);
        assert_eq!(result.file_results_omitted, 1);
        assert_eq!(result.apparent_bytes_scanned, 60);
        let root = result
            .largest_folders_by_recursive_bytes
            .iter()
            .find(|item| item.path.as_str() == managed.to_string_lossy().as_ref())
            .expect("root folder");
        assert!(root.immediate_complete);
        assert!(!root.recursive_complete);
    }

    #[test]
    fn analysis_enforces_concurrency_and_supports_cancellation() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        fs::create_dir(&managed).expect("managed directory");
        for index in 0..100 {
            fs::write(managed.join(format!("{index}.bin")), [0_u8]).expect("analysis fixture");
        }
        let mut limits = analysis_limits();
        limits.max_concurrent_jobs = 1;
        limits.per_entry_delay = Duration::from_millis(5);
        let manager = StorageAnalysisManager::with_limits(vec![managed.clone()], limits)
            .expect("analysis manager");
        let first = manager.start(managed.to_str().unwrap()).expect("first job");
        assert!(manager.start(managed.to_str().unwrap()).is_err());
        manager.cancel(&first.job_id).expect("cancel job");
        let status = wait_for_analysis(&manager, &first.job_id);
        assert_eq!(status.state, StorageAnalysisJobState::Cancelled);
        assert!(status.cancel_requested);
        let result = status.result.expect("partial result");
        assert_eq!(
            result.stop_reason,
            Some(StorageAnalysisStopReason::Cancelled)
        );
        assert!(result.truncated);
    }

    #[test]
    fn analysis_duration_and_entry_limits_are_honest() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let managed = temporary.path().join("managed");
        fs::create_dir(&managed).expect("managed directory");
        for index in 0..10 {
            fs::write(managed.join(format!("{index}.bin")), [0_u8]).expect("analysis fixture");
        }
        let mut entry_limits = analysis_limits();
        entry_limits.max_entries = 2;
        let manager = StorageAnalysisManager::with_limits(vec![managed.clone()], entry_limits)
            .expect("entry limited manager");
        let result = wait_for_analysis(
            &manager,
            &manager.start(managed.to_str().unwrap()).unwrap().job_id,
        )
        .result
        .expect("entry limited result");
        assert_eq!(
            result.stop_reason,
            Some(StorageAnalysisStopReason::EntryLimit)
        );
        assert_eq!(result.limits.max_entries, 2);

        let mut duration_limits = analysis_limits();
        duration_limits.max_duration = Duration::from_millis(10);
        duration_limits.per_entry_delay = Duration::from_millis(10);
        let manager = StorageAnalysisManager::with_limits(vec![managed.clone()], duration_limits)
            .expect("duration limited manager");
        let result = wait_for_analysis(
            &manager,
            &manager.start(managed.to_str().unwrap()).unwrap().job_id,
        )
        .result
        .expect("duration limited result");
        assert_eq!(
            result.stop_reason,
            Some(StorageAnalysisStopReason::DurationLimit)
        );
        assert!(result.truncated);
    }

    #[test]
    fn analysis_result_is_trimmed_to_its_serialized_response_bound() {
        let job_id = Uuid::new_v4();
        let root = Path::new("/srv/media");
        let files = (0_u64..128)
            .map(|index| StorageAnalysisFile {
                path: format!("/srv/media/{index}-{}", "x".repeat(220)),
                name: format!("{index}-{}", "x".repeat(220)),
                kind: StorageAnalysisEntryKind::File,
                bytes: index,
                allocated_bytes: index,
            })
            .collect();
        let mut result = StorageAnalysisResult {
            root_path: root.to_string_lossy().into_owned(),
            apparent_bytes_scanned: 8_128,
            allocated_bytes_scanned: 8_128,
            truncated: false,
            stop_reason: None,
            errors: StorageAnalysisErrors::default(),
            skipped: StorageAnalysisSkipped::default(),
            file_results_omitted: 0,
            recursive_folder_results_omitted: 0,
            immediate_folder_results_omitted: 0,
            response_results_omitted: 0,
            largest_files: files,
            largest_folders_by_recursive_bytes: Vec::new(),
            largest_folders_by_immediate_bytes: Vec::new(),
            limits: StorageAnalysisAppliedLimits {
                mode: StorageAnalysisMode::Quick,
                max_duration_ms: 30_000,
                max_entries: 250_000,
                max_depth: 64,
                max_results_per_list: 128,
                max_response_bytes: 4_096,
                stay_on_target_filesystem: true,
                follows_symbolic_links: false,
            },
        };
        fit_storage_analysis_response(&mut result, job_id, root, 1, 2, false, 4_096);
        let status = StorageAnalysisStatus {
            job_id: job_id.hyphenated().to_string(),
            requested_path: root.to_string_lossy().into_owned(),
            state: StorageAnalysisJobState::Complete,
            progress: StorageAnalysisProgress {
                percent: Some(100),
                entry_budget_percent: 1,
                duration_budget_percent: 1,
                entries_scanned: 128,
                files_scanned: 128,
                directories_scanned: 1,
                bytes_scanned: 8_128,
                allocated_bytes_scanned: 8_128,
            },
            created_unix_ms: 1,
            started_unix_ms: Some(1),
            finished_unix_ms: Some(2),
            cancel_requested: false,
            result: Some(result.clone()),
            error: None,
        };
        assert!(serde_json::to_vec(&status).unwrap().len() <= 4_096);
        assert!(!result.truncated);
        assert!(result.response_results_omitted > 0);
    }

    #[test]
    fn analysis_job_ids_require_the_exact_opaque_wire_form() {
        let id = Uuid::new_v4().hyphenated().to_string();
        assert_eq!(
            parse_analysis_job_id(&id).unwrap().hyphenated().to_string(),
            id
        );
        assert!(parse_analysis_job_id(&id.to_uppercase()).is_err());
        assert!(parse_analysis_job_id(&id.replace('-', "")).is_err());
        assert!(parse_analysis_job_id(&format!("{{{id}}}")).is_err());
    }
}
