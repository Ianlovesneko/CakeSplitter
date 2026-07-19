use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use cakesplitter_format::{
    CakeManifest, MAX_FILENAME_BYTES, MAX_MANIFEST_BYTES, MAX_SLICE_COUNT, SliceEntry,
    parse_manifest_json,
};
use cakesplitter_integrity::Sha256State;
use serde::{Deserialize, Serialize};

use super::resumable::{is_reparse_point, validate_no_reparse_components};
use super::{
    CancellationToken, CoreError, DirectoryFingerprint, NativeFileIdentity, PackageInspection,
    SourceFingerprint, check_cancelled, file_identity, io_error, source_state,
    validate_existing_directory, validate_existing_regular_file,
};

pub const PACKAGE_BINDING_VERSION: u32 = 1;
pub const MAX_PACKAGE_DIRECTORY_ENTRIES: usize = 65_536;
pub const MAX_PACKAGE_SLICE_CANDIDATES: usize = MAX_SLICE_COUNT as usize + 1_024;
pub const MAX_PACKAGE_DIAGNOSTIC_ENTRIES: usize = 1_024;
pub const MAX_PACKAGE_ENTRY_FILENAME_BYTES: usize = MAX_FILENAME_BYTES;
pub const MAX_PACKAGE_ENUMERATION_METADATA_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_PACKAGE_INSPECTION_SERIALIZED_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_PACKAGE_RENDERED_DIAGNOSTIC_ROWS: usize = 20;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSliceBinding {
    pub filename: String,
    pub identity: Option<SourceFingerprint>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageBinding {
    pub binding_version: u32,
    pub manifest_identity: SourceFingerprint,
    pub manifest_sha256: String,
    pub package_directory: DirectoryFingerprint,
    pub manifest: CakeManifest,
    pub slices: Vec<PackageSliceBinding>,
    pub membership_sha256: String,
}

#[derive(Debug)]
struct PackageInventory {
    slice_names: Vec<String>,
    manifest_paths: Vec<PathBuf>,
}

pub(crate) struct BoundPackage {
    manifest_path: PathBuf,
    manifest_handle: File,
    guard: PackageDirectoryGuard,
    binding: PackageBinding,
    inventory: PackageInventory,
}

pub(crate) struct BoundSlice {
    pub file: File,
    pub path: PathBuf,
    expected: SourceFingerprint,
    expected_sha256: String,
}

pub fn find_package_manifest(
    directory: &Path,
    cancellation: &CancellationToken,
) -> Result<PathBuf, CoreError> {
    let guard = PackageDirectoryGuard::acquire(directory)?;
    let inventory = enumerate_package_directory(directory, cancellation)?;
    guard.revalidate()?;
    if inventory.manifest_paths.len() != 1 {
        return Err(CoreError::ResumeRejected(
            "the selected folder must contain exactly one Cake Manifest".to_owned(),
        ));
    }
    Ok(inventory.manifest_paths[0].clone())
}

pub fn capture_package_binding(
    manifest_path: &Path,
    cancellation: &CancellationToken,
) -> Result<PackageBinding, CoreError> {
    let directory = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let guard = PackageDirectoryGuard::acquire(directory)?;
    let (binding, _, manifest_handle, inventory) =
        capture_with_guard(manifest_path, &guard, cancellation)?;
    guard.revalidate()?;
    verify_manifest_handle(manifest_path, &manifest_handle, &binding, cancellation)?;
    let final_inventory = enumerate_package_directory(directory, cancellation)?;
    if membership_sha256(&final_inventory.slice_names) != binding.membership_sha256 {
        return Err(CoreError::PackageIdentityChanged(
            manifest_path.to_path_buf(),
        ));
    }
    drop(inventory);
    validate_package_binding_shape(&binding)?;
    Ok(binding)
}

pub fn validate_package_binding_shape(binding: &PackageBinding) -> Result<(), CoreError> {
    if binding.binding_version != PACKAGE_BINDING_VERSION
        || binding.slices.len() != binding.manifest.slices.len()
        || binding.manifest_sha256.len() != 64
        || binding.membership_sha256.len() != 64
        || !binding
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !binding
            .membership_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CoreError::ResumeRejected(
            "the durable Cake Package binding is invalid".to_owned(),
        ));
    }
    binding.manifest.validate()?;
    for (expected, actual) in binding.manifest.slices.iter().zip(&binding.slices) {
        if expected.filename != actual.filename {
            return Err(CoreError::ResumeRejected(
                "the durable Slice binding order is invalid".to_owned(),
            ));
        }
    }
    Ok(())
}

pub fn inspect_package_bound(
    manifest_path: &Path,
    verify_hashes: bool,
    expected: &PackageBinding,
    cancellation: &CancellationToken,
) -> Result<PackageInspection, CoreError> {
    BoundPackage::open(manifest_path, expected, cancellation)?.inspect(verify_hashes, cancellation)
}

pub(crate) fn bounded_unexpected_slice_names(
    directory: &Path,
    expected: &HashSet<String>,
    cancellation: &CancellationToken,
) -> Result<Vec<String>, CoreError> {
    let inventory = enumerate_package_directory(directory, cancellation)?;
    unexpected_from_names(inventory.slice_names, expected)
}

impl BoundPackage {
    pub(crate) fn open(
        manifest_path: &Path,
        expected: &PackageBinding,
        cancellation: &CancellationToken,
    ) -> Result<Self, CoreError> {
        validate_package_binding_shape(expected)?;
        let directory = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let guard = PackageDirectoryGuard::acquire(directory)?;
        let (actual, _manifest, manifest_handle, inventory) =
            capture_with_guard(manifest_path, &guard, cancellation)?;
        if &actual != expected {
            return Err(CoreError::PackageIdentityChanged(
                manifest_path.to_path_buf(),
            ));
        }
        Ok(Self {
            manifest_path: manifest_path.to_path_buf(),
            manifest_handle,
            guard,
            binding: expected.clone(),
            inventory,
        })
    }

    pub(crate) fn manifest(&self) -> &CakeManifest {
        &self.binding.manifest
    }

    pub(crate) fn inspect(
        &mut self,
        verify_hashes: bool,
        cancellation: &CancellationToken,
    ) -> Result<PackageInspection, CoreError> {
        self.guard.revalidate()?;
        let directory = self
            .manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let expected_names: HashSet<_> = self
            .binding
            .manifest
            .slices
            .iter()
            .map(|slice| slice.filename.clone())
            .collect();
        let mut missing = Vec::new();
        let mut corrupted = Vec::new();
        let mut found_slice_count = 0_u64;

        for (slice, bound) in self
            .binding
            .manifest
            .slices
            .iter()
            .zip(&self.binding.slices)
        {
            check_cancelled(cancellation)?;
            let Some(_) = bound.identity else {
                missing.push(slice.filename.clone());
                continue;
            };
            found_slice_count += 1;
            let mut opened = self.open_slice(slice)?;
            let actual_size = opened
                .file
                .metadata()
                .map_err(|error| io_error(&opened.path, error))?
                .len();
            if actual_size != slice.size {
                corrupted.push(slice.filename.clone());
            } else if verify_hashes {
                let actual = hash_exact_file(&mut opened.file, &opened.path, cancellation)?;
                if actual != slice.sha256 {
                    corrupted.push(slice.filename.clone());
                }
            }
            opened.validate_after(None)?;
        }

        let unexpected = bounded_unexpected_slice_names(directory, &expected_names, cancellation)?;
        self.revalidate(cancellation)?;
        let verified =
            verify_hashes && missing.is_empty() && corrupted.is_empty() && unexpected.is_empty();
        Ok(PackageInspection {
            manifest: self.binding.manifest.clone(),
            expected_slice_count: self.binding.manifest.slice_count,
            found_slice_count,
            missing,
            corrupted,
            unexpected,
            verified,
        })
    }

    pub(crate) fn open_slice(&self, slice: &SliceEntry) -> Result<BoundSlice, CoreError> {
        self.guard.revalidate()?;
        let index = usize::try_from(slice.index.saturating_sub(1))
            .map_err(|_| CoreError::PackageIdentityChanged(self.manifest_path.clone()))?;
        let bound = self
            .binding
            .slices
            .get(index)
            .ok_or_else(|| CoreError::PackageIdentityChanged(self.manifest_path.clone()))?;
        if bound.filename != slice.filename {
            return Err(CoreError::PackageIdentityChanged(
                self.manifest_path.clone(),
            ));
        }
        let expected = bound
            .identity
            .clone()
            .ok_or_else(|| CoreError::PackageIdentityChanged(self.manifest_path.clone()))?;
        let path = self
            .manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&slice.filename);
        validate_existing_regular_file(&path)?;
        let file = open_guarded_file(&path).map_err(|error| io_error(&path, error))?;
        if fingerprint_handle(&file, &path)? != expected {
            return Err(CoreError::PackageIdentityChanged(
                self.manifest_path.clone(),
            ));
        }
        Ok(BoundSlice {
            file,
            path,
            expected,
            expected_sha256: slice.sha256.clone(),
        })
    }

    pub(crate) fn revalidate(&mut self, cancellation: &CancellationToken) -> Result<(), CoreError> {
        self.guard.revalidate()?;
        verify_manifest_handle(
            &self.manifest_path,
            &self.manifest_handle,
            &self.binding,
            cancellation,
        )?;
        let directory = self
            .manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let inventory = enumerate_package_directory(directory, cancellation)?;
        if membership_sha256(&inventory.slice_names) != self.binding.membership_sha256 {
            return Err(CoreError::PackageIdentityChanged(
                self.manifest_path.clone(),
            ));
        }
        self.inventory = inventory;
        Ok(())
    }
}

impl BoundSlice {
    pub(crate) fn into_file(self) -> File {
        self.file
    }

    pub(crate) fn validate_after(&mut self, actual_sha256: Option<&str>) -> Result<(), CoreError> {
        if fingerprint_handle(&self.file, &self.path)? != self.expected
            || actual_sha256.is_some_and(|actual| actual != self.expected_sha256)
        {
            return Err(CoreError::PackageIdentityChanged(self.path.clone()));
        }
        Ok(())
    }
}

fn capture_with_guard(
    manifest_path: &Path,
    guard: &PackageDirectoryGuard,
    cancellation: &CancellationToken,
) -> Result<(PackageBinding, CakeManifest, File, PackageInventory), CoreError> {
    guard.revalidate()?;
    let (manifest, manifest_handle, manifest_identity, manifest_sha256) =
        open_manifest(manifest_path, cancellation)?;
    let directory = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let inventory = enumerate_package_directory(directory, cancellation)?;
    let present: HashSet<_> = inventory.slice_names.iter().cloned().collect();
    let mut slices = Vec::with_capacity(manifest.slices.len());
    for slice in &manifest.slices {
        check_cancelled(cancellation)?;
        let identity = if present.contains(&slice.filename) {
            let path = directory.join(&slice.filename);
            validate_existing_regular_file(&path)?;
            let file = open_guarded_file(&path).map_err(|error| io_error(&path, error))?;
            Some(fingerprint_handle(&file, &path)?)
        } else {
            None
        };
        slices.push(PackageSliceBinding {
            filename: slice.filename.clone(),
            identity,
        });
    }
    guard.revalidate()?;
    let binding = PackageBinding {
        binding_version: PACKAGE_BINDING_VERSION,
        manifest_identity,
        manifest_sha256,
        package_directory: guard.fingerprint.clone(),
        manifest: manifest.clone(),
        slices,
        membership_sha256: membership_sha256(&inventory.slice_names),
    };
    Ok((binding, manifest, manifest_handle, inventory))
}

fn open_manifest(
    manifest_path: &Path,
    cancellation: &CancellationToken,
) -> Result<(CakeManifest, File, SourceFingerprint, String), CoreError> {
    validate_existing_regular_file(manifest_path)?;
    let mut file =
        open_guarded_file(manifest_path).map_err(|error| io_error(manifest_path, error))?;
    let identity = fingerprint_handle(&file, manifest_path)?;
    if identity.len > MAX_MANIFEST_BYTES as u64 {
        return Err(cakesplitter_format::ManifestError::ManifestTooLarge.into());
    }
    check_cancelled(cancellation)?;
    let mut json = String::new();
    file.by_ref()
        .take(MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_string(&mut json)
        .map_err(|error| io_error(manifest_path, error))?;
    if json.len() > MAX_MANIFEST_BYTES {
        return Err(cakesplitter_format::ManifestError::ManifestTooLarge.into());
    }
    let manifest = parse_manifest_json(&json)?;
    if fingerprint_handle(&file, manifest_path)? != identity {
        return Err(CoreError::PackageIdentityChanged(
            manifest_path.to_path_buf(),
        ));
    }
    let mut hasher = Sha256State::new();
    hasher.update(json.as_bytes());
    Ok((manifest, file, identity, hasher.finish()))
}

fn verify_manifest_handle(
    manifest_path: &Path,
    handle: &File,
    expected: &PackageBinding,
    cancellation: &CancellationToken,
) -> Result<(), CoreError> {
    if fingerprint_handle(handle, manifest_path)? != expected.manifest_identity {
        return Err(CoreError::PackageIdentityChanged(
            manifest_path.to_path_buf(),
        ));
    }
    let mut copy = handle
        .try_clone()
        .map_err(|error| io_error(manifest_path, error))?;
    copy.seek(SeekFrom::Start(0))
        .map_err(|error| io_error(manifest_path, error))?;
    if hash_exact_file(&mut copy, manifest_path, cancellation)? != expected.manifest_sha256 {
        return Err(CoreError::PackageIdentityChanged(
            manifest_path.to_path_buf(),
        ));
    }
    Ok(())
}

fn enumerate_package_directory(
    directory: &Path,
    cancellation: &CancellationToken,
) -> Result<PackageInventory, CoreError> {
    validate_existing_directory(directory)?;
    let mut budget = EnumerationBudget::default();
    let mut slice_names = Vec::new();
    let mut manifest_paths = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| io_error(directory, error))? {
        check_cancelled(cancellation)?;
        let entry = entry.map_err(|error| io_error(directory, error))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| CoreError::NonUtf8Filename)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| io_error(&path, error))?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(CoreError::UnsafeFilesystemPath(path));
        }
        let is_slice = metadata.is_file() && name.ends_with(".slice");
        budget.record(&name, is_slice)?;
        if !metadata.is_file() {
            continue;
        }
        if is_slice {
            slice_names.push(name.clone());
        }
        if name.ends_with(".cake.json") {
            manifest_paths.push(path);
        }
    }
    slice_names.sort();
    manifest_paths.sort();
    Ok(PackageInventory {
        slice_names,
        manifest_paths,
    })
}

#[derive(Default)]
struct EnumerationBudget {
    entry_count: usize,
    candidate_count: usize,
    metadata_bytes: usize,
}

impl EnumerationBudget {
    fn record(&mut self, name: &str, is_slice: bool) -> Result<(), CoreError> {
        if name.len() > MAX_PACKAGE_ENTRY_FILENAME_BYTES {
            return Err(CoreError::PackageEnumerationLimit {
                resource: "entry filename byte length",
                maximum: MAX_PACKAGE_ENTRY_FILENAME_BYTES,
            });
        }
        self.entry_count =
            self.entry_count
                .checked_add(1)
                .ok_or(CoreError::PackageEnumerationLimit {
                    resource: "directory entry count",
                    maximum: MAX_PACKAGE_DIRECTORY_ENTRIES,
                })?;
        if self.entry_count > MAX_PACKAGE_DIRECTORY_ENTRIES {
            return Err(CoreError::PackageEnumerationLimit {
                resource: "directory entry count",
                maximum: MAX_PACKAGE_DIRECTORY_ENTRIES,
            });
        }
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(name.len().saturating_add(64))
            .ok_or(CoreError::PackageEnumerationLimit {
                resource: "enumeration metadata bytes",
                maximum: MAX_PACKAGE_ENUMERATION_METADATA_BYTES,
            })?;
        if self.metadata_bytes > MAX_PACKAGE_ENUMERATION_METADATA_BYTES {
            return Err(CoreError::PackageEnumerationLimit {
                resource: "enumeration metadata bytes",
                maximum: MAX_PACKAGE_ENUMERATION_METADATA_BYTES,
            });
        }
        if is_slice {
            self.candidate_count =
                self.candidate_count
                    .checked_add(1)
                    .ok_or(CoreError::PackageEnumerationLimit {
                        resource: "candidate Slice count",
                        maximum: MAX_PACKAGE_SLICE_CANDIDATES,
                    })?;
            if self.candidate_count > MAX_PACKAGE_SLICE_CANDIDATES {
                return Err(CoreError::PackageEnumerationLimit {
                    resource: "candidate Slice count",
                    maximum: MAX_PACKAGE_SLICE_CANDIDATES,
                });
            }
        }
        Ok(())
    }
}

fn unexpected_from_names(
    names: Vec<String>,
    expected: &HashSet<String>,
) -> Result<Vec<String>, CoreError> {
    let mut unexpected = Vec::new();
    for name in names {
        if !expected.contains(&name) {
            if unexpected.len() >= MAX_PACKAGE_DIAGNOSTIC_ENTRIES {
                return Err(CoreError::PackageEnumerationLimit {
                    resource: "unexpected Slice diagnostic count",
                    maximum: MAX_PACKAGE_DIAGNOSTIC_ENTRIES,
                });
            }
            unexpected.push(name);
        }
    }
    Ok(unexpected)
}

fn membership_sha256(names: &[String]) -> String {
    let mut hasher = Sha256State::new();
    for name in names {
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
    }
    hasher.finish()
}

fn hash_exact_file(
    file: &mut File,
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<String, CoreError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| io_error(path, error))?;
    let mut buffer = vec![0_u8; super::DEFAULT_BUFFER_SIZE];
    let mut hasher = Sha256State::new();
    loop {
        check_cancelled(cancellation)?;
        let read = file
            .read(&mut buffer)
            .map_err(|error| io_error(path, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish())
}

fn fingerprint_handle(file: &File, path: &Path) -> Result<SourceFingerprint, CoreError> {
    let state = source_state(file).map_err(|error| io_error(path, error))?;
    let modified_unix_nanos = state
        .modified
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    Ok(SourceFingerprint {
        identity: NativeFileIdentity::from(state.identity),
        len: state.len,
        modified_unix_nanos,
    })
}

struct PackageDirectoryGuard {
    directory: PathBuf,
    components: Vec<GuardedDirectory>,
    fingerprint: DirectoryFingerprint,
}

struct GuardedDirectory {
    path: PathBuf,
    handle: File,
    identity: NativeFileIdentity,
}

impl PackageDirectoryGuard {
    fn acquire(directory: &Path) -> Result<Self, CoreError> {
        validate_existing_directory(directory)?;
        let mut components = Vec::new();
        for path in directory.ancestors().collect::<Vec<_>>().into_iter().rev() {
            if path.as_os_str().is_empty() {
                continue;
            }
            let metadata = fs::symlink_metadata(path)
                .map_err(|_| CoreError::PackageIdentityChanged(directory.to_path_buf()))?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || is_reparse_point(&metadata)
            {
                return Err(CoreError::UnsafeFilesystemPath(path.to_path_buf()));
            }
            let handle = open_guarded_directory(path)
                .map_err(|_| CoreError::PackageIdentityChanged(directory.to_path_buf()))?;
            let identity = NativeFileIdentity::from(
                file_identity(&handle)
                    .map_err(|_| CoreError::PackageIdentityChanged(directory.to_path_buf()))?,
            );
            components.push(GuardedDirectory {
                path: path.to_path_buf(),
                handle,
                identity,
            });
        }
        let fingerprint = DirectoryFingerprint {
            identity: components
                .last()
                .ok_or_else(|| CoreError::UnsafeFilesystemPath(directory.to_path_buf()))?
                .identity,
        };
        let guard = Self {
            directory: directory.to_path_buf(),
            components,
            fingerprint,
        };
        guard.revalidate()?;
        Ok(guard)
    }

    fn revalidate(&self) -> Result<(), CoreError> {
        let failure = || CoreError::PackageIdentityChanged(self.directory.clone());
        validate_no_reparse_components(&self.directory).map_err(|_| failure())?;
        for component in &self.components {
            let retained =
                NativeFileIdentity::from(file_identity(&component.handle).map_err(|_| failure())?);
            if retained != component.identity {
                return Err(failure());
            }
            let metadata = fs::symlink_metadata(&component.path).map_err(|_| failure())?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || is_reparse_point(&metadata)
            {
                return Err(failure());
            }
            let current = open_directory(&component.path).map_err(|_| failure())?;
            if NativeFileIdentity::from(file_identity(&current).map_err(|_| failure())?)
                != component.identity
            {
                return Err(failure());
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
fn open_guarded_file(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ_WRITE: u32 = 0x0000_0003;
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ_WRITE)
        .open(path)
}

#[cfg(not(windows))]
fn open_guarded_file(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(windows)]
fn open_directory(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_SHARE_ALL: u32 = 0x0000_0007;
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

#[cfg(windows)]
fn open_guarded_directory(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_SHARE_READ_WRITE: u32 = 0x0000_0003;
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

#[cfg(not(windows))]
fn open_directory(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(not(windows))]
fn open_guarded_directory(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumeration_budget_accepts_exact_entry_and_candidate_limits_then_rejects_one_more() {
        let mut entries = EnumerationBudget::default();
        for _ in 0..MAX_PACKAGE_DIRECTORY_ENTRIES {
            entries.record("x", false).unwrap();
        }
        assert!(matches!(
            entries.record("x", false),
            Err(CoreError::PackageEnumerationLimit {
                resource: "directory entry count",
                ..
            })
        ));

        let mut candidates = EnumerationBudget::default();
        for _ in 0..MAX_PACKAGE_SLICE_CANDIDATES {
            candidates.record("x.slice", true).unwrap();
        }
        assert!(matches!(
            candidates.record("x.slice", true),
            Err(CoreError::PackageEnumerationLimit {
                resource: "candidate Slice count",
                ..
            })
        ));
    }

    #[test]
    fn enumeration_budget_enforces_filename_and_metadata_byte_limits() {
        let mut budget = EnumerationBudget::default();
        budget
            .record(&"x".repeat(MAX_PACKAGE_ENTRY_FILENAME_BYTES), false)
            .unwrap();
        assert!(matches!(
            budget.record(&"x".repeat(MAX_PACKAGE_ENTRY_FILENAME_BYTES + 1), false),
            Err(CoreError::PackageEnumerationLimit {
                resource: "entry filename byte length",
                ..
            })
        ));

        let mut metadata = EnumerationBudget {
            metadata_bytes: MAX_PACKAGE_ENUMERATION_METADATA_BYTES - 65,
            ..Default::default()
        };
        metadata.record("x", false).unwrap();
        assert!(matches!(
            metadata.record("x", false),
            Err(CoreError::PackageEnumerationLimit {
                resource: "enumeration metadata bytes",
                ..
            })
        ));
    }

    #[test]
    fn unexpected_diagnostics_accept_exact_limit_and_reject_one_more() {
        let expected = HashSet::new();
        let exact = (0..MAX_PACKAGE_DIAGNOSTIC_ENTRIES)
            .map(|index| format!("unexpected-{index}.slice"))
            .collect::<Vec<_>>();
        assert_eq!(
            unexpected_from_names(exact.clone(), &expected)
                .unwrap()
                .len(),
            MAX_PACKAGE_DIAGNOSTIC_ENTRIES
        );
        let mut excess = exact;
        excess.push("one-too-many.slice".to_owned());
        assert!(matches!(
            unexpected_from_names(excess, &expected),
            Err(CoreError::PackageEnumerationLimit {
                resource: "unexpected Slice diagnostic count",
                ..
            })
        ));
    }
}
