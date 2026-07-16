//! Streaming split, merge, inspect, and verify operations for Cake Packages.

use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use cakesplitter_format::{
    CakeManifest, FORMAT_IDENTIFIER, FORMAT_VERSION, MAX_MANIFEST_BYTES, MAX_SAFE_INTEGER,
    MAX_SLICE_COUNT, ManifestError, OriginalFile, SliceEntry, expected_slice_count,
    parse_manifest_json, slice_filename, slice_index_width, validate_portable_filename,
};
use cakesplitter_integrity::Sha256State;
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

pub const DEFAULT_BUFFER_SIZE: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    volume: u64,
    file: u64,
}

#[derive(Debug)]
struct StagedOutput {
    partial_path: PathBuf,
    final_path: PathBuf,
    identity: FileIdentity,
    expected_size: u64,
    expected_sha256: String,
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone)]
pub struct SplitOptions {
    pub slice_size: u64,
    pub output_dir: PathBuf,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub operation: &'static str,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub current_slice: u64,
    pub slice_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInspection {
    pub manifest: CakeManifest,
    pub expected_slice_count: u64,
    pub found_slice_count: u64,
    pub missing: Vec<String>,
    pub corrupted: Vec<String>,
    pub unexpected: Vec<String>,
    pub verified: bool,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("manifest JSON is invalid: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("manifest validation failed: {0}")]
    InvalidManifest(#[from] ManifestError),
    #[error("input must be a regular file: {0}")]
    InvalidInput(PathBuf),
    #[error("slice size must be between 1 and {MAX_SAFE_INTEGER} bytes")]
    InvalidSliceSize,
    #[error("slice count {actual} exceeds the supported maximum of {maximum}")]
    SliceLimit { actual: u64, maximum: u64 },
    #[error("output already exists: {0}")]
    Collision(PathBuf),
    #[error("operation cancelled")]
    Cancelled,
    #[error("source file changed while it was being processed")]
    SourceChanged,
    #[error("package is incomplete; missing: {0:?}")]
    MissingSlices(Vec<String>),
    #[error("package contains unexpected slices: {0:?}")]
    UnexpectedSlices(Vec<String>),
    #[error("package contains corrupted slices: {0:?}")]
    CorruptedSlices(Vec<String>),
    #[error("rebuilt SHA-256 does not match the manifest; expected {expected}, found {actual}")]
    FinalHashMismatch { expected: String, actual: String },
    #[error("file name is not valid UTF-8")]
    NonUtf8Filename,
    #[error("staged output identity changed before verified publication: {0}")]
    StagedIdentityChanged(PathBuf),
    #[error("staged output content changed before verified publication: {0}")]
    StagedContentChanged(PathBuf),
    #[error("atomic no-replace finalization is not supported for: {0}")]
    AtomicFinalizationUnsupported(PathBuf),
}

impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "io_error",
            Self::InvalidJson(_) => "invalid_json",
            Self::InvalidManifest(_) => "invalid_manifest",
            Self::InvalidInput(_) => "invalid_input",
            Self::InvalidSliceSize => "invalid_slice_size",
            Self::SliceLimit { .. } => "resource_limit",
            Self::Collision(_) => "output_collision",
            Self::Cancelled => "cancelled",
            Self::SourceChanged => "source_changed",
            Self::MissingSlices(_) => "missing_slices",
            Self::UnexpectedSlices(_) => "unexpected_slices",
            Self::CorruptedSlices(_) => "corrupted_slices",
            Self::FinalHashMismatch { .. } => "final_hash_mismatch",
            Self::NonUtf8Filename => "non_utf8_filename",
            Self::StagedIdentityChanged(_) => "staged_identity_changed",
            Self::StagedContentChanged(_) => "staged_content_changed",
            Self::AtomicFinalizationUnsupported(_) => "atomic_finalization_unsupported",
        }
    }
}

pub fn split_file(input: &Path, options: &SplitOptions) -> Result<PathBuf, CoreError> {
    split_file_with_progress(input, options, |_| {})
}

pub fn split_file_with_progress<F>(
    input: &Path,
    options: &SplitOptions,
    mut on_progress: F,
) -> Result<PathBuf, CoreError>
where
    F: FnMut(Progress),
{
    if options.slice_size == 0 || options.slice_size > MAX_SAFE_INTEGER {
        return Err(CoreError::InvalidSliceSize);
    }
    let metadata = fs::metadata(input).map_err(|source| io_error(input, source))?;
    if !metadata.is_file() {
        return Err(CoreError::InvalidInput(input.to_path_buf()));
    }
    fs::create_dir_all(&options.output_dir)
        .map_err(|source| io_error(&options.output_dir, source))?;
    let original_filename = input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(CoreError::NonUtf8Filename)?
        .to_owned();
    validate_portable_filename(&original_filename)?;

    let total_size = metadata.len();
    if total_size > MAX_SAFE_INTEGER {
        return Err(ManifestError::UnsafeInteger.into());
    }
    let slice_count = expected_slice_count(total_size, options.slice_size);
    if slice_count > MAX_SLICE_COUNT {
        return Err(CoreError::SliceLimit {
            actual: slice_count,
            maximum: MAX_SLICE_COUNT,
        });
    }
    let width = slice_index_width(slice_count);
    let manifest_filename = format!("{original_filename}.cake.json");
    validate_portable_filename(&manifest_filename)?;
    let manifest_path = options.output_dir.join(manifest_filename);
    ensure_absent(&manifest_path)?;
    ensure_absent(&partial_path(&manifest_path))?;

    let mut paths = Vec::with_capacity(slice_count as usize);
    for index in 1..=slice_count {
        let filename = slice_filename(&original_filename, index, width);
        validate_portable_filename(&filename)?;
        let final_path = options.output_dir.join(filename);
        let partial = partial_path(&final_path);
        ensure_absent(&final_path)?;
        ensure_absent(&partial)?;
        paths.push((partial, final_path));
    }

    {
        let mut input_file = File::open(input).map_err(|source| io_error(input, source))?;
        let mut buffer = vec![0_u8; DEFAULT_BUFFER_SIZE];
        let mut original_hasher = Sha256State::new();
        let mut slices = Vec::with_capacity(slice_count as usize);
        let mut staged_outputs = Vec::with_capacity(slice_count as usize);
        let mut bytes_processed = 0_u64;

        for (position, (partial, final_path)) in paths.iter().enumerate() {
            check_cancelled(&options.cancellation)?;
            let index = position as u64 + 1;
            let offset = bytes_processed;
            let expected_size = (total_size - offset).min(options.slice_size);
            let mut remaining = expected_size;
            let mut slice_hasher = Sha256State::new();
            let mut output = create_new(partial)?;
            while remaining > 0 {
                check_cancelled(&options.cancellation)?;
                let limit = remaining.min(buffer.len() as u64) as usize;
                let read = input_file
                    .read(&mut buffer[..limit])
                    .map_err(|source| io_error(input, source))?;
                if read == 0 {
                    return Err(CoreError::SourceChanged);
                }
                output
                    .write_all(&buffer[..read])
                    .map_err(|source| io_error(partial, source))?;
                original_hasher.update(&buffer[..read]);
                slice_hasher.update(&buffer[..read]);
                remaining -= read as u64;
                bytes_processed += read as u64;
                on_progress(Progress {
                    operation: "split",
                    bytes_processed,
                    total_bytes: total_size,
                    current_slice: index,
                    slice_count,
                });
            }
            output.flush().map_err(|source| io_error(partial, source))?;
            output
                .sync_all()
                .map_err(|source| io_error(partial, source))?;
            let identity = file_identity(&output).map_err(|source| io_error(partial, source))?;
            let slice_sha256 = slice_hasher.finish();
            staged_outputs.push(StagedOutput {
                partial_path: partial.clone(),
                final_path: final_path.clone(),
                identity,
                expected_size,
                expected_sha256: slice_sha256.clone(),
            });
            slices.push(SliceEntry {
                index,
                filename: final_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or(CoreError::NonUtf8Filename)?
                    .to_owned(),
                offset,
                size: expected_size,
                sha256: slice_sha256,
            });
        }

        let mut probe = [0_u8; 1];
        if input_file
            .read(&mut probe)
            .map_err(|source| io_error(input, source))?
            != 0
        {
            return Err(CoreError::SourceChanged);
        }

        let manifest = CakeManifest {
            format: FORMAT_IDENTIFIER.to_owned(),
            version: FORMAT_VERSION.to_owned(),
            package_id: Uuid::new_v4().to_string(),
            created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
            original: OriginalFile {
                filename: original_filename,
                size: total_size,
                sha256: original_hasher.finish(),
            },
            target_slice_size: options.slice_size,
            slice_count,
            slices,
        };
        manifest.validate()?;

        let manifest_partial = partial_path(&manifest_path);
        let encoded = serde_json::to_vec_pretty(&manifest)?;
        let mut output = create_new(&manifest_partial)?;
        output
            .write_all(&encoded)
            .map_err(|source| io_error(&manifest_partial, source))?;
        output
            .write_all(b"\n")
            .map_err(|source| io_error(&manifest_partial, source))?;
        output
            .flush()
            .map_err(|source| io_error(&manifest_partial, source))?;
        output
            .sync_all()
            .map_err(|source| io_error(&manifest_partial, source))?;
        let manifest_identity =
            file_identity(&output).map_err(|source| io_error(&manifest_partial, source))?;
        let mut manifest_hasher = Sha256State::new();
        manifest_hasher.update(&encoded);
        manifest_hasher.update(b"\n");
        let manifest_staged = StagedOutput {
            partial_path: manifest_partial.clone(),
            final_path: manifest_path.clone(),
            identity: manifest_identity,
            expected_size: encoded.len() as u64 + 1,
            expected_sha256: manifest_hasher.finish(),
        };
        drop(output);

        for staged in &staged_outputs {
            finalize_staged_output(staged, &options.cancellation)?;
        }
        finalize_staged_output(&manifest_staged, &options.cancellation)?;
        Ok(manifest_path.clone())
    }
}

pub fn load_manifest(path: &Path) -> Result<CakeManifest, CoreError> {
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    if file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .len()
        > MAX_MANIFEST_BYTES as u64
    {
        return Err(ManifestError::ManifestTooLarge.into());
    }
    let mut json = String::new();
    file.take(MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_string(&mut json)
        .map_err(|source| io_error(path, source))?;
    parse_manifest_json(&json).map_err(CoreError::from)
}

pub fn inspect_package(
    manifest_path: &Path,
    verify_hashes: bool,
    cancellation: &CancellationToken,
) -> Result<PackageInspection, CoreError> {
    let manifest = load_manifest(manifest_path)?;
    let directory = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let expected: HashSet<_> = manifest
        .slices
        .iter()
        .map(|slice| slice.filename.clone())
        .collect();
    let mut missing = Vec::new();
    let mut corrupted = Vec::new();
    let mut found_slice_count = 0_u64;
    let mut buffer = vec![0_u8; DEFAULT_BUFFER_SIZE];

    for slice in &manifest.slices {
        check_cancelled(cancellation)?;
        let path = directory.join(&slice.filename);
        if !path.is_file() {
            missing.push(slice.filename.clone());
            continue;
        }
        found_slice_count += 1;
        let metadata = fs::metadata(&path).map_err(|source| io_error(&path, source))?;
        if metadata.len() != slice.size {
            corrupted.push(slice.filename.clone());
            continue;
        }
        if verify_hashes {
            let actual = hash_file(&path, &mut buffer, cancellation)?;
            if actual != slice.sha256 {
                corrupted.push(slice.filename.clone());
            }
        }
    }

    let mut unexpected = Vec::new();
    for entry in fs::read_dir(directory).map_err(|source| io_error(directory, source))? {
        let entry = entry.map_err(|source| io_error(directory, source))?;
        if !entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".slice") && !expected.contains(&name) {
            unexpected.push(name);
        }
    }
    unexpected.sort();
    let verified =
        verify_hashes && missing.is_empty() && corrupted.is_empty() && unexpected.is_empty();
    Ok(PackageInspection {
        expected_slice_count: manifest.slice_count,
        found_slice_count,
        manifest,
        missing,
        corrupted,
        unexpected,
        verified,
    })
}

pub fn verify_package(
    manifest_path: &Path,
    cancellation: &CancellationToken,
) -> Result<PackageInspection, CoreError> {
    inspect_package(manifest_path, true, cancellation)
}

pub fn merge_package(
    manifest_path: &Path,
    output_path: &Path,
    cancellation: &CancellationToken,
) -> Result<(), CoreError> {
    merge_package_with_progress(manifest_path, output_path, cancellation, |_| {})
}

pub fn merge_package_with_progress<F>(
    manifest_path: &Path,
    output_path: &Path,
    cancellation: &CancellationToken,
    mut on_progress: F,
) -> Result<(), CoreError>
where
    F: FnMut(Progress),
{
    let manifest = load_manifest(manifest_path)?;
    let inspection = inspect_package(manifest_path, true, cancellation)?;
    if !inspection.missing.is_empty() {
        return Err(CoreError::MissingSlices(inspection.missing));
    }
    if !inspection.corrupted.is_empty() {
        return Err(CoreError::CorruptedSlices(inspection.corrupted));
    }
    if !inspection.unexpected.is_empty() {
        return Err(CoreError::UnexpectedSlices(inspection.unexpected));
    }
    ensure_absent(output_path)?;
    let partial = partial_path(output_path);
    ensure_absent(&partial)?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    }

    {
        let mut output = create_new(&partial)?;
        let mut original_hasher = Sha256State::new();
        let mut buffer = vec![0_u8; DEFAULT_BUFFER_SIZE];
        let directory = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let mut bytes_processed = 0_u64;

        for slice in &manifest.slices {
            check_cancelled(cancellation)?;
            let path = directory.join(&slice.filename);
            let mut input = File::open(&path).map_err(|source| io_error(&path, source))?;
            let mut remaining = slice.size;
            while remaining > 0 {
                check_cancelled(cancellation)?;
                let limit = remaining.min(buffer.len() as u64) as usize;
                let read = input
                    .read(&mut buffer[..limit])
                    .map_err(|source| io_error(&path, source))?;
                if read == 0 {
                    return Err(CoreError::CorruptedSlices(vec![slice.filename.clone()]));
                }
                output
                    .write_all(&buffer[..read])
                    .map_err(|source| io_error(&partial, source))?;
                original_hasher.update(&buffer[..read]);
                remaining -= read as u64;
                bytes_processed += read as u64;
                on_progress(Progress {
                    operation: "merge",
                    bytes_processed,
                    total_bytes: manifest.original.size,
                    current_slice: slice.index,
                    slice_count: manifest.slice_count,
                });
            }
            let mut probe = [0_u8; 1];
            if input
                .read(&mut probe)
                .map_err(|source| io_error(&path, source))?
                != 0
            {
                return Err(CoreError::CorruptedSlices(vec![slice.filename.clone()]));
            }
        }
        output
            .flush()
            .map_err(|source| io_error(&partial, source))?;
        output
            .sync_all()
            .map_err(|source| io_error(&partial, source))?;
        let actual = original_hasher.finish();
        if actual != manifest.original.sha256 {
            return Err(CoreError::FinalHashMismatch {
                expected: manifest.original.sha256,
                actual,
            });
        }
        let identity = file_identity(&output).map_err(|source| io_error(&partial, source))?;
        drop(output);
        finalize_staged_output(
            &StagedOutput {
                partial_path: partial.clone(),
                final_path: output_path.to_path_buf(),
                identity,
                expected_size: manifest.original.size,
                expected_sha256: manifest.original.sha256.clone(),
            },
            cancellation,
        )?;
        Ok(())
    }
}

fn hash_file(
    path: &Path,
    buffer: &mut [u8],
    cancellation: &CancellationToken,
) -> Result<String, CoreError> {
    let mut file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut hasher = Sha256State::new();
    loop {
        check_cancelled(cancellation)?;
        let read = file.read(buffer).map_err(|source| io_error(path, source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish())
}
fn ensure_absent(path: &Path) -> Result<(), CoreError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(CoreError::Collision(path.to_path_buf())),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn create_new(path: &Path) -> Result<File, CoreError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                CoreError::Collision(path.to_path_buf())
            } else {
                io_error(path, source)
            }
        })
}

fn partial_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(".partial");
    PathBuf::from(value)
}

fn finalize_staged_output(
    staged: &StagedOutput,
    cancellation: &CancellationToken,
) -> Result<(), CoreError> {
    check_cancelled(cancellation)?;
    let mut partial = File::open(&staged.partial_path)
        .map_err(|source| io_error(&staged.partial_path, source))?;
    let metadata = partial
        .metadata()
        .map_err(|source| io_error(&staged.partial_path, source))?;
    if file_identity(&partial).map_err(|source| io_error(&staged.partial_path, source))?
        != staged.identity
    {
        return Err(CoreError::StagedIdentityChanged(
            staged.partial_path.clone(),
        ));
    }
    if metadata.len() != staged.expected_size
        || hash_reader(&mut partial, &staged.partial_path, cancellation)? != staged.expected_sha256
    {
        return Err(CoreError::StagedContentChanged(staged.partial_path.clone()));
    }

    atomic_rename_no_replace(&staged.partial_path, &staged.final_path).map_err(|source| {
        if is_collision_error(&source) {
            CoreError::Collision(staged.final_path.clone())
        } else if source.kind() == std::io::ErrorKind::Unsupported {
            CoreError::AtomicFinalizationUnsupported(staged.final_path.clone())
        } else {
            io_error(&staged.final_path, source)
        }
    })?;

    let mut published =
        File::open(&staged.final_path).map_err(|source| io_error(&staged.final_path, source))?;
    let published_metadata = published
        .metadata()
        .map_err(|source| io_error(&staged.final_path, source))?;
    if file_identity(&published).map_err(|source| io_error(&staged.final_path, source))?
        != staged.identity
    {
        return Err(CoreError::StagedIdentityChanged(staged.final_path.clone()));
    }
    if published_metadata.len() != staged.expected_size
        || hash_reader(&mut published, &staged.final_path, cancellation)? != staged.expected_sha256
    {
        return Err(CoreError::StagedContentChanged(staged.final_path.clone()));
    }
    Ok(())
}

fn hash_reader(
    file: &mut File,
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<String, CoreError> {
    let mut buffer = vec![0_u8; DEFAULT_BUFFER_SIZE];
    let mut hasher = Sha256State::new();
    loop {
        check_cancelled(cancellation)?;
        let read = file
            .read(&mut buffer)
            .map_err(|source| io_error(path, source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish())
}

#[cfg(unix)]
fn file_identity(file: &File) -> std::io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(FileIdentity {
        volume: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(windows)]
fn file_identity(file: &File) -> std::io::Result<FileIdentity> {
    use std::{ffi::c_void, mem::MaybeUninit, os::windows::io::AsRawHandle};

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    struct ByHandleFileInformation {
        file_attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    unsafe extern "system" {
        fn GetFileInformationByHandle(
            file: *mut c_void,
            information: *mut ByHandleFileInformation,
        ) -> i32;
    }

    let mut information = MaybeUninit::<ByHandleFileInformation>::uninit();
    // SAFETY: `file` owns a valid Windows handle and `information` points to a
    // correctly sized writable structure for the duration of this call.
    let result =
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a successful GetFileInformationByHandle call initializes the
    // complete BY_HANDLE_FILE_INFORMATION-compatible structure.
    let information = unsafe { information.assume_init() };
    Ok(FileIdentity {
        volume: u64::from(information.volume_serial_number),
        file: (u64::from(information.file_index_high) << 32)
            | u64::from(information.file_index_low),
    })
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File) -> std::io::Result<FileIdentity> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "stable file identity is unavailable on this platform",
    ))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn atomic_rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both paths are NUL-terminated C strings and remain alive for the
    // syscall. RENAME_NOREPLACE is the required atomic collision contract.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_vendor = "apple")]
fn atomic_rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both paths are valid C strings and RENAME_EXCL provides the
    // platform's atomic no-replace rename contract.
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn atomic_rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both UTF-16 path buffers are NUL terminated and live for the
    // call. A zero flag set deliberately omits MOVEFILE_REPLACE_EXISTING.
    let result = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android")),
    not(target_vendor = "apple")
))]
fn atomic_rename_no_replace(_from: &Path, _to: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace rename is unavailable on this platform",
    ))
}

#[cfg(not(any(unix, windows)))]
fn atomic_rename_no_replace(_from: &Path, _to: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace rename is unavailable on this platform",
    ))
}

fn is_collision_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::AlreadyExists
        || matches!(error.raw_os_error(), Some(17 | 80 | 183))
}

fn check_cancelled(token: &CancellationToken) -> Result<(), CoreError> {
    if token.is_cancelled() {
        Err(CoreError::Cancelled)
    } else {
        Ok(())
    }
}

fn io_error(path: impl AsRef<Path>, source: std::io::Error) -> CoreError {
    CoreError::Io {
        path: path.as_ref().to_path_buf(),
        source,
    }
}
