use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use cakesplitter_format::{
    CakeManifest, FORMAT_IDENTIFIER, FORMAT_VERSION, MAX_SAFE_INTEGER, MAX_SLICE_COUNT,
    OriginalFile, SliceEntry, expected_slice_count, slice_filename, slice_index_width,
    validate_portable_filename,
};
use cakesplitter_integrity::Sha256State;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    CancellationToken, CoreError, DEFAULT_BUFFER_SIZE, FileIdentity, PackageBinding, Progress,
    SourceState, StagedOutput, check_cancelled, create_new, ensure_absent, ensure_source_unchanged,
    file_identity, finalize_staged_output, hash_reader, inspect_package, io_error, load_manifest,
    package_binding::BoundPackage, source_state,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeFileIdentity {
    pub volume: u64,
    pub file: u64,
}

impl From<FileIdentity> for NativeFileIdentity {
    fn from(value: FileIdentity) -> Self {
        Self {
            volume: value.volume,
            file: value.file,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFingerprint {
    pub identity: NativeFileIdentity,
    pub len: u64,
    pub modified_unix_nanos: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryFingerprint {
    pub identity: NativeFileIdentity,
}

/// Keeps the selected output directory and each replaceable ancestor open for
/// the lifetime of a resumable publication. On Windows, the handles omit
/// delete sharing, preventing directory rename/replacement while publication
/// is in progress. Boundary revalidation also proves that the textual path
/// still resolves to every originally opened filesystem object.
struct DestinationIdentityGuard {
    destination: PathBuf,
    components: Vec<GuardedDirectory>,
    fingerprint: DirectoryFingerprint,
}

struct GuardedDirectory {
    path: PathBuf,
    handle: File,
    identity: NativeFileIdentity,
}

impl DestinationIdentityGuard {
    fn acquire(destination: &Path) -> Result<Self, CoreError> {
        if let Err(error) = validate_existing_directory(destination) {
            return match error {
                CoreError::UnsafeFilesystemPath(_) => Err(error),
                _ => Err(CoreError::DestinationIdentityChanged(
                    destination.to_path_buf(),
                )),
            };
        }
        let mut components = Vec::new();
        for path in destination
            .ancestors()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if path.as_os_str().is_empty() {
                continue;
            }
            let metadata = fs::symlink_metadata(path)
                .map_err(|_| CoreError::DestinationIdentityChanged(destination.to_path_buf()))?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || is_reparse_point(&metadata)
            {
                return Err(CoreError::UnsafeFilesystemPath(path.to_path_buf()));
            }
            let handle = open_guarded_directory(path)
                .map_err(|_| CoreError::DestinationIdentityChanged(destination.to_path_buf()))?;
            let identity = file_identity(&handle)
                .map_err(|_| CoreError::DestinationIdentityChanged(destination.to_path_buf()))?
                .into();
            components.push(GuardedDirectory {
                path: path.to_path_buf(),
                handle,
                identity,
            });
        }
        let fingerprint = DirectoryFingerprint {
            identity: components
                .last()
                .ok_or_else(|| CoreError::UnsafeFilesystemPath(destination.to_path_buf()))?
                .identity,
        };
        let guard = Self {
            destination: destination.to_path_buf(),
            components,
            fingerprint,
        };
        guard.revalidate()?;
        Ok(guard)
    }

    fn fingerprint(&self) -> DirectoryFingerprint {
        self.fingerprint.clone()
    }

    fn revalidate(&self) -> Result<(), CoreError> {
        let failure = || CoreError::DestinationIdentityChanged(self.destination.clone());
        validate_no_reparse_components(&self.destination).map_err(|_| failure())?;
        for component in &self.components {
            let retained: NativeFileIdentity = file_identity(&component.handle)
                .map_err(|_| failure())?
                .into();
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
            let current_identity: NativeFileIdentity =
                file_identity(&current).map_err(|_| failure())?.into();
            if current_identity != component.identity {
                return Err(failure());
            }
        }
        Ok(())
    }
}

/// Native authority retained while a desktop export publishes into a selected
/// directory. The authority holds the selected directory and its ancestors
/// open and revalidates their identities before each security-sensitive write.
pub struct DirectoryIdentityAuthority {
    guard: DestinationIdentityGuard,
}

impl DirectoryIdentityAuthority {
    pub fn acquire(
        destination: &Path,
        expected: Option<&DirectoryFingerprint>,
    ) -> Result<Self, CoreError> {
        let guard = DestinationIdentityGuard::acquire(destination)?;
        if expected.is_some_and(|expected| expected != &guard.fingerprint()) {
            return Err(CoreError::DestinationIdentityChanged(
                destination.to_path_buf(),
            ));
        }
        Ok(Self { guard })
    }

    pub fn revalidate(&self) -> Result<(), CoreError> {
        self.guard.revalidate()
    }

    pub fn fingerprint(&self) -> DirectoryFingerprint {
        self.guard.fingerprint()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialCheckpoint {
    pub filename: String,
    pub identity: NativeFileIdentity,
    pub verified_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SliceCheckpoint {
    pub entry: SliceEntry,
    pub identity: NativeFileIdentity,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitResumeData {
    pub source: SourceFingerprint,
    pub output_directory: DirectoryFingerprint,
    pub baseline_sha256: String,
    pub completed: Vec<SliceCheckpoint>,
    pub active_partial: Option<PartialCheckpoint>,
    #[serde(default)]
    pub published_manifest_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResumableSplitOptions {
    pub task_id: String,
    pub package_id: String,
    pub created_at: String,
    pub slice_size: u64,
    pub output_dir: PathBuf,
    pub cancellation: CancellationToken,
    pub resume: Option<SplitResumeData>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SplitCheckpointEvent {
    Baseline {
        source: SourceFingerprint,
        output_directory: DirectoryFingerprint,
        baseline_sha256: String,
    },
    PartialCreated {
        partial: PartialCheckpoint,
    },
    SliceCompleted {
        checkpoint: SliceCheckpoint,
    },
    PartialCleared,
    ManifestPublished {
        filename: String,
        sha256: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeResumeData {
    pub output_directory: DirectoryFingerprint,
    pub partial: PartialCheckpoint,
    pub completed_slices: u64,
    pub completed_bytes: u64,
    #[serde(default)]
    pub published_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResumableMergeOptions {
    pub task_id: String,
    pub cancellation: CancellationToken,
    pub resume: Option<MergeResumeData>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MergeCheckpointEvent {
    PartialCreated {
        output_directory: DirectoryFingerprint,
        partial: PartialCheckpoint,
    },
    SliceBoundary {
        completed_slices: u64,
        completed_bytes: u64,
    },
    Published {
        filename: String,
        sha256: String,
    },
}

/// Returns the zero-based byte offset and byte length for a one-based Slice.
///
/// This planner is independent from file I/O so large-file arithmetic can be
/// validated without allocating or touching the logical source size.
pub fn planned_slice_range(
    total_size: u64,
    slice_size: u64,
    index: u64,
) -> Result<(u64, u64), CoreError> {
    if total_size > MAX_SAFE_INTEGER {
        return Err(cakesplitter_format::ManifestError::UnsafeInteger.into());
    }
    if slice_size == 0 || slice_size > MAX_SAFE_INTEGER {
        return Err(CoreError::InvalidSliceSize);
    }
    let slice_count = expected_slice_count(total_size, slice_size);
    if slice_count > MAX_SLICE_COUNT {
        return Err(CoreError::SliceLimit {
            actual: slice_count,
            maximum: MAX_SLICE_COUNT,
        });
    }
    if index == 0 || index > slice_count {
        return Err(CoreError::ResumeRejected(
            "Slice index is outside the processing plan".to_owned(),
        ));
    }
    let offset = index
        .checked_sub(1)
        .and_then(|value| value.checked_mul(slice_size))
        .ok_or_else(|| CoreError::ResumeRejected("Slice offset overflow".to_owned()))?;
    let remaining = total_size
        .checked_sub(offset)
        .ok_or_else(|| CoreError::ResumeRejected("Slice offset exceeds source size".to_owned()))?;
    Ok((offset, remaining.min(slice_size)))
}

pub fn split_file_resumable_with_progress<P, C>(
    input: &Path,
    options: &ResumableSplitOptions,
    mut on_progress: P,
    mut on_checkpoint: C,
) -> Result<PathBuf, CoreError>
where
    P: FnMut(Progress),
    C: FnMut(SplitCheckpointEvent),
{
    validate_task_id(&options.task_id)?;
    validate_package_id(&options.package_id)?;
    if options.slice_size == 0 || options.slice_size > MAX_SAFE_INTEGER {
        return Err(CoreError::InvalidSliceSize);
    }
    validate_existing_regular_file(input)?;
    let destination_guard = DestinationIdentityGuard::acquire(&options.output_dir)?;

    let mut source = File::open(input).map_err(|error| io_error(input, error))?;
    let state = source_state(&source).map_err(|_| CoreError::SourceChanged)?;
    let source_fingerprint = fingerprint_source(&state);
    let output_directory = destination_guard.fingerprint();
    let original_filename = input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(CoreError::NonUtf8Filename)?
        .to_owned();
    validate_portable_filename(&original_filename)?;
    if state.len > MAX_SAFE_INTEGER {
        return Err(cakesplitter_format::ManifestError::UnsafeInteger.into());
    }
    let slice_count = expected_slice_count(state.len, options.slice_size);
    if slice_count > MAX_SLICE_COUNT {
        return Err(CoreError::SliceLimit {
            actual: slice_count,
            maximum: MAX_SLICE_COUNT,
        });
    }
    let width = slice_index_width(slice_count);
    let manifest_filename = format!("{original_filename}.cake.json");
    validate_portable_filename(&manifest_filename)?;
    let manifest_path = options.output_dir.join(&manifest_filename);

    destination_guard.revalidate()?;
    ensure_source_unchanged(input, &source, &state)?;
    let baseline_sha256 = hash_reader(&mut source, input, &options.cancellation)?;
    ensure_source_unchanged(input, &source, &state)?;

    let mut completed = if let Some(resume) = &options.resume {
        if resume.source != source_fingerprint {
            return Err(CoreError::ResumeRejected(
                "the selected source identity or metadata changed".to_owned(),
            ));
        }
        if resume.output_directory != output_directory {
            return Err(CoreError::DestinationIdentityChanged(
                options.output_dir.clone(),
            ));
        }
        if resume.baseline_sha256 != baseline_sha256 {
            return Err(CoreError::ResumeRejected(
                "the selected source content changed".to_owned(),
            ));
        }
        let mut resumed_completed = resume.completed.clone();
        if let Some(partial) = &resume.active_partial {
            destination_guard.revalidate()?;
            let active_path = options.output_dir.join(&partial.filename);
            if active_path.exists() {
                remove_owned_partial(&active_path, partial.identity)?;
                on_checkpoint(SplitCheckpointEvent::PartialCleared);
            } else {
                let next_index = resumed_completed.len() as u64 + 1;
                let recovered_final = if next_index <= slice_count {
                    options
                        .output_dir
                        .join(slice_filename(&original_filename, next_index, width))
                } else {
                    manifest_path.clone()
                };
                if filename_of(&task_partial_path(&recovered_final, &options.task_id)?)?
                    != partial.filename
                {
                    return Err(CoreError::ResumeRejected(
                        "the active partial name does not match the processing plan".to_owned(),
                    ));
                }
                validate_existing_regular_file(&recovered_final)?;
                let mut recovered = File::open(&recovered_final)
                    .map_err(|error| io_error(&recovered_final, error))?;
                let identity: NativeFileIdentity = file_identity(&recovered)
                    .map_err(|error| io_error(&recovered_final, error))?
                    .into();
                if identity != partial.identity {
                    return Err(CoreError::ResumeRejected(
                        "the atomically published file identity changed".to_owned(),
                    ));
                }
                if next_index <= slice_count {
                    let (offset, expected_size) =
                        planned_slice_range(state.len, options.slice_size, next_index)?;
                    if recovered
                        .metadata()
                        .map_err(|error| io_error(&recovered_final, error))?
                        .len()
                        != expected_size
                    {
                        return Err(CoreError::ResumeRejected(
                            "the published Slice length changed".to_owned(),
                        ));
                    }
                    let sha256 =
                        hash_reader(&mut recovered, &recovered_final, &options.cancellation)?;
                    if hash_range(
                        &mut source,
                        input,
                        offset,
                        expected_size,
                        &options.cancellation,
                    )? != sha256
                    {
                        return Err(CoreError::ResumeRejected(
                            "the published Slice no longer matches its source range".to_owned(),
                        ));
                    }
                    let checkpoint = SliceCheckpoint {
                        entry: SliceEntry {
                            index: next_index,
                            filename: filename_of(&recovered_final)?,
                            offset,
                            size: expected_size,
                            sha256,
                        },
                        identity,
                    };
                    resumed_completed.push(checkpoint.clone());
                    on_checkpoint(SplitCheckpointEvent::SliceCompleted { checkpoint });
                } else {
                    let published_stream_sha256 = validate_completed_slices(
                        &mut source,
                        input,
                        &SplitValidationPlan {
                            output_dir: &options.output_dir,
                            original_filename: &original_filename,
                            slice_size: options.slice_size,
                            slice_count,
                            width,
                        },
                        &resumed_completed,
                        &options.cancellation,
                    )?;
                    if published_stream_sha256 != baseline_sha256 {
                        return Err(CoreError::SourceChanged);
                    }
                    let manifest_sha256 = validate_recovered_manifest(
                        &manifest_path,
                        options,
                        &original_filename,
                        state.len,
                        &baseline_sha256,
                        &resumed_completed,
                    )?;
                    ensure_source_unchanged(input, &source, &state)?;
                    destination_guard.revalidate()?;
                    on_checkpoint(SplitCheckpointEvent::ManifestPublished {
                        filename: manifest_filename,
                        sha256: manifest_sha256,
                    });
                    return Ok(manifest_path);
                }
            }
        }
        if manifest_path.exists() {
            destination_guard.revalidate()?;
            let expected_sha256 = resume
                .published_manifest_sha256
                .as_deref()
                .ok_or_else(|| CoreError::Collision(manifest_path.clone()))?;
            let published_stream_sha256 = validate_completed_slices(
                &mut source,
                input,
                &SplitValidationPlan {
                    output_dir: &options.output_dir,
                    original_filename: &original_filename,
                    slice_size: options.slice_size,
                    slice_count,
                    width,
                },
                &resumed_completed,
                &options.cancellation,
            )?;
            if published_stream_sha256 != baseline_sha256 {
                return Err(CoreError::SourceChanged);
            }
            let actual_sha256 = validate_recovered_manifest(
                &manifest_path,
                options,
                &original_filename,
                state.len,
                &baseline_sha256,
                &resumed_completed,
            )?;
            if actual_sha256 != expected_sha256 {
                return Err(CoreError::ResumeRejected(
                    "the published Manifest content changed".to_owned(),
                ));
            }
            ensure_source_unchanged(input, &source, &state)?;
            destination_guard.revalidate()?;
            return Ok(manifest_path);
        }
        destination_guard.revalidate()?;
        let published_stream_sha256 = validate_completed_slices(
            &mut source,
            input,
            &SplitValidationPlan {
                output_dir: &options.output_dir,
                original_filename: &original_filename,
                slice_size: options.slice_size,
                slice_count,
                width,
            },
            &resumed_completed,
            &options.cancellation,
        )?;
        if resumed_completed.len() as u64 == slice_count
            && published_stream_sha256 != baseline_sha256
        {
            return Err(CoreError::SourceChanged);
        }
        resumed_completed
    } else {
        destination_guard.revalidate()?;
        ensure_absent(&manifest_path)?;
        for index in 1..=slice_count {
            let final_path =
                options
                    .output_dir
                    .join(slice_filename(&original_filename, index, width));
            ensure_absent(&final_path)?;
            ensure_absent(&task_partial_path(&final_path, &options.task_id)?)?;
        }
        on_checkpoint(SplitCheckpointEvent::Baseline {
            source: source_fingerprint.clone(),
            output_directory: output_directory.clone(),
            baseline_sha256: baseline_sha256.clone(),
        });
        destination_guard.revalidate()?;
        Vec::new()
    };
    destination_guard.revalidate()?;
    ensure_absent(&manifest_path)?;

    let mut buffer = vec![0_u8; DEFAULT_BUFFER_SIZE];
    let mut bytes_processed = completed.iter().map(|item| item.entry.size).sum::<u64>();
    for index in completed.len() as u64 + 1..=slice_count {
        check_cancelled(&options.cancellation)?;
        destination_guard.revalidate()?;
        let (offset, expected_size) = planned_slice_range(state.len, options.slice_size, index)?;
        let filename = slice_filename(&original_filename, index, width);
        let final_path = options.output_dir.join(&filename);
        ensure_absent(&final_path)?;
        let partial_path = task_partial_path(&final_path, &options.task_id)?;
        ensure_absent(&partial_path)?;
        source
            .seek(SeekFrom::Start(offset))
            .map_err(|error| io_error(input, error))?;

        let mut output = create_new(&partial_path)?;
        let identity: NativeFileIdentity = file_identity(&output)
            .map_err(|error| io_error(&partial_path, error))?
            .into();
        on_checkpoint(SplitCheckpointEvent::PartialCreated {
            partial: PartialCheckpoint {
                filename: filename_of(&partial_path)?,
                identity,
                verified_bytes: 0,
            },
        });
        destination_guard.revalidate()?;

        let mut hasher = Sha256State::new();
        let mut remaining = expected_size;
        while remaining > 0 {
            check_cancelled(&options.cancellation)?;
            let limit = remaining.min(buffer.len() as u64) as usize;
            let read = source
                .read(&mut buffer[..limit])
                .map_err(|error| io_error(input, error))?;
            if read == 0 {
                return Err(CoreError::SourceChanged);
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| io_error(&partial_path, error))?;
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
            bytes_processed += read as u64;
            on_progress(Progress {
                operation: "split",
                bytes_processed,
                total_bytes: state.len,
                current_slice: index,
                slice_count,
            });
        }
        output
            .flush()
            .map_err(|error| io_error(&partial_path, error))?;
        output
            .sync_all()
            .map_err(|error| io_error(&partial_path, error))?;
        ensure_source_unchanged(input, &source, &state)?;
        let sha256 = hasher.finish();
        let staged = StagedOutput {
            partial_path: partial_path.clone(),
            final_path: final_path.clone(),
            identity: FileIdentity {
                volume: identity.volume,
                file: identity.file,
            },
            expected_size,
            expected_sha256: sha256.clone(),
        };
        drop(output);
        destination_guard.revalidate()?;
        finalize_staged_output(&staged, &options.cancellation)?;
        destination_guard.revalidate()?;
        let checkpoint = SliceCheckpoint {
            entry: SliceEntry {
                index,
                filename,
                offset,
                size: expected_size,
                sha256,
            },
            identity,
        };
        completed.push(checkpoint.clone());
        on_checkpoint(SplitCheckpointEvent::SliceCompleted { checkpoint });
        destination_guard.revalidate()?;
    }

    source
        .seek(SeekFrom::Start(0))
        .map_err(|error| io_error(input, error))?;
    let final_source_hash = hash_reader(&mut source, input, &options.cancellation)?;
    ensure_source_unchanged(input, &source, &state)?;
    if final_source_hash != baseline_sha256 {
        return Err(CoreError::SourceChanged);
    }

    let published_stream_sha256 = validate_completed_slices(
        &mut source,
        input,
        &SplitValidationPlan {
            output_dir: &options.output_dir,
            original_filename: &original_filename,
            slice_size: options.slice_size,
            slice_count,
            width,
        },
        &completed,
        &options.cancellation,
    )
    .map_err(|error| match error {
        CoreError::ResumeRejected(_) => CoreError::SourceChanged,
        other => other,
    })?;
    if published_stream_sha256 != baseline_sha256 {
        return Err(CoreError::SourceChanged);
    }
    destination_guard.revalidate()?;

    let manifest = CakeManifest {
        format: FORMAT_IDENTIFIER.to_owned(),
        version: FORMAT_VERSION.to_owned(),
        package_id: options.package_id.clone(),
        created_at: options.created_at.clone(),
        original: OriginalFile {
            filename: original_filename,
            size: state.len,
            sha256: baseline_sha256,
        },
        target_slice_size: options.slice_size,
        slice_count,
        slices: completed.iter().map(|item| item.entry.clone()).collect(),
    };
    manifest.validate()?;
    let encoded = serde_json::to_vec_pretty(&manifest)?;
    let manifest_partial = task_partial_path(&manifest_path, &options.task_id)?;
    destination_guard.revalidate()?;
    ensure_absent(&manifest_partial)?;
    let mut output = create_new(&manifest_partial)?;
    let identity: NativeFileIdentity = file_identity(&output)
        .map_err(|error| io_error(&manifest_partial, error))?
        .into();
    on_checkpoint(SplitCheckpointEvent::PartialCreated {
        partial: PartialCheckpoint {
            filename: filename_of(&manifest_partial)?,
            identity,
            verified_bytes: 0,
        },
    });
    destination_guard.revalidate()?;
    output
        .write_all(&encoded)
        .and_then(|_| output.write_all(b"\n"))
        .and_then(|_| output.flush())
        .and_then(|_| output.sync_all())
        .map_err(|error| io_error(&manifest_partial, error))?;
    let mut manifest_hasher = Sha256State::new();
    manifest_hasher.update(&encoded);
    manifest_hasher.update(b"\n");
    let manifest_sha256 = manifest_hasher.finish();
    let staged = StagedOutput {
        partial_path: manifest_partial,
        final_path: manifest_path.clone(),
        identity: FileIdentity {
            volume: identity.volume,
            file: identity.file,
        },
        expected_size: encoded.len() as u64 + 1,
        expected_sha256: manifest_sha256.clone(),
    };
    drop(output);
    destination_guard.revalidate()?;
    finalize_staged_output(&staged, &options.cancellation)?;
    destination_guard.revalidate()?;
    if ensure_source_unchanged(input, &source, &state).is_err() {
        remove_owned_partial(&manifest_path, identity)?;
        return Err(CoreError::SourceChanged);
    }
    on_checkpoint(SplitCheckpointEvent::ManifestPublished {
        filename: manifest_filename,
        sha256: manifest_sha256,
    });
    destination_guard.revalidate()?;
    Ok(manifest_path)
}

pub fn merge_package_resumable_with_progress<P, C>(
    manifest_path: &Path,
    output_path: &Path,
    options: &ResumableMergeOptions,
    on_progress: P,
    on_checkpoint: C,
) -> Result<(), CoreError>
where
    P: FnMut(Progress),
    C: FnMut(MergeCheckpointEvent),
{
    merge_package_resumable_inner(
        manifest_path,
        output_path,
        options,
        None,
        on_progress,
        on_checkpoint,
    )
}

pub fn merge_package_resumable_bound_with_progress<P, C>(
    manifest_path: &Path,
    output_path: &Path,
    options: &ResumableMergeOptions,
    package_binding: &PackageBinding,
    on_progress: P,
    on_checkpoint: C,
) -> Result<(), CoreError>
where
    P: FnMut(Progress),
    C: FnMut(MergeCheckpointEvent),
{
    merge_package_resumable_inner(
        manifest_path,
        output_path,
        options,
        Some(package_binding),
        on_progress,
        on_checkpoint,
    )
}

fn merge_package_resumable_inner<P, C>(
    manifest_path: &Path,
    output_path: &Path,
    options: &ResumableMergeOptions,
    package_binding: Option<&PackageBinding>,
    mut on_progress: P,
    mut on_checkpoint: C,
) -> Result<(), CoreError>
where
    P: FnMut(Progress),
    C: FnMut(MergeCheckpointEvent),
{
    validate_task_id(&options.task_id)?;
    validate_existing_regular_file(manifest_path)?;
    let mut bound_package = package_binding
        .map(|binding| BoundPackage::open(manifest_path, binding, &options.cancellation))
        .transpose()?;
    let manifest = if let Some(package) = bound_package.as_ref() {
        package.manifest().clone()
    } else {
        load_manifest(manifest_path)?
    };
    let inspection = if let Some(package) = bound_package.as_mut() {
        package.inspect(true, &options.cancellation)?
    } else {
        inspect_package(manifest_path, true, &options.cancellation)?
    };
    if !inspection.missing.is_empty() {
        return Err(CoreError::MissingSlices(inspection.missing));
    }
    if !inspection.corrupted.is_empty() {
        return Err(CoreError::CorruptedSlices(inspection.corrupted));
    }
    if !inspection.unexpected.is_empty() {
        return Err(CoreError::UnexpectedSlices(inspection.unexpected));
    }
    let parent = output_path
        .parent()
        .ok_or_else(|| CoreError::UnsafeFilesystemPath(output_path.to_path_buf()))?;
    let destination_guard = DestinationIdentityGuard::acquire(parent)?;
    validate_portable_filename(
        output_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CoreError::NonUtf8Filename)?,
    )?;
    let output_directory = destination_guard.fingerprint();
    let partial_path = task_partial_path(output_path, &options.task_id)?;

    if output_path.exists() {
        destination_guard.revalidate()?;
        let resume = options
            .resume
            .as_ref()
            .ok_or_else(|| CoreError::Collision(output_path.to_path_buf()))?;
        if resume.output_directory != output_directory {
            return Err(CoreError::DestinationIdentityChanged(parent.to_path_buf()));
        }
        if resume.partial.filename != filename_of(&partial_path)?
            || resume.completed_slices != manifest.slice_count
            || resume.completed_bytes != manifest.original.size
        {
            return Err(CoreError::ResumeRejected(
                "the published output checkpoint does not match the task plan".to_owned(),
            ));
        }
        validate_existing_regular_file(output_path)?;
        let mut recovered =
            File::open(output_path).map_err(|error| io_error(output_path, error))?;
        let identity: NativeFileIdentity = file_identity(&recovered)
            .map_err(|error| io_error(output_path, error))?
            .into();
        if identity != resume.partial.identity
            || recovered
                .metadata()
                .map_err(|error| io_error(output_path, error))?
                .len()
                != manifest.original.size
        {
            return Err(CoreError::ResumeRejected(
                "the published rebuilt output identity or size changed".to_owned(),
            ));
        }
        verify_partial_prefix(
            &mut recovered,
            output_path,
            &manifest.slices,
            resume.completed_slices,
            &options.cancellation,
        )?;
        recovered
            .seek(SeekFrom::Start(0))
            .map_err(|error| io_error(output_path, error))?;
        let actual = hash_reader(&mut recovered, output_path, &options.cancellation)?;
        if actual != manifest.original.sha256
            || resume
                .published_sha256
                .as_ref()
                .is_some_and(|expected| expected != &actual)
        {
            return Err(CoreError::ResumeRejected(
                "the published rebuilt output content changed".to_owned(),
            ));
        }
        if let Some(package) = bound_package.as_mut() {
            package.revalidate(&options.cancellation)?;
        }
        destination_guard.revalidate()?;
        return Ok(());
    }
    destination_guard.revalidate()?;
    ensure_absent(output_path)?;

    let (mut output, partial_identity, mut completed_slices, mut completed_bytes) =
        if let Some(resume) = &options.resume {
            if resume.output_directory != output_directory {
                return Err(CoreError::DestinationIdentityChanged(parent.to_path_buf()));
            }
            if resume.partial.filename != filename_of(&partial_path)? {
                return Err(CoreError::ResumeRejected(
                    "the partial-output name changed".to_owned(),
                ));
            }
            validate_existing_regular_file(&partial_path)?;
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&partial_path)
                .map_err(|error| io_error(&partial_path, error))?;
            let identity: NativeFileIdentity = file_identity(&file)
                .map_err(|error| io_error(&partial_path, error))?
                .into();
            if identity != resume.partial.identity {
                return Err(CoreError::ResumeRejected(
                    "the partial-output identity changed".to_owned(),
                ));
            }
            let expected_bytes = manifest
                .slices
                .iter()
                .take(resume.completed_slices as usize)
                .map(|slice| slice.size)
                .sum::<u64>();
            if expected_bytes != resume.completed_bytes
                || file
                    .metadata()
                    .map_err(|error| io_error(&partial_path, error))?
                    .len()
                    < expected_bytes
            {
                return Err(CoreError::ResumeRejected(
                    "the partial-output length does not match its verified boundary".to_owned(),
                ));
            }
            file.set_len(expected_bytes)
                .map_err(|error| io_error(&partial_path, error))?;
            verify_partial_prefix(
                &mut file,
                &partial_path,
                &manifest.slices,
                resume.completed_slices,
                &options.cancellation,
            )?;
            file.seek(SeekFrom::Start(expected_bytes))
                .map_err(|error| io_error(&partial_path, error))?;
            (file, identity, resume.completed_slices, expected_bytes)
        } else {
            destination_guard.revalidate()?;
            ensure_absent(&partial_path)?;
            let file = create_new(&partial_path)?;
            let identity: NativeFileIdentity = file_identity(&file)
                .map_err(|error| io_error(&partial_path, error))?
                .into();
            on_checkpoint(MergeCheckpointEvent::PartialCreated {
                output_directory: output_directory.clone(),
                partial: PartialCheckpoint {
                    filename: filename_of(&partial_path)?,
                    identity,
                    verified_bytes: 0,
                },
            });
            destination_guard.revalidate()?;
            (file, identity, 0, 0)
        };

    let directory = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut buffer = vec![0_u8; DEFAULT_BUFFER_SIZE];
    for slice in manifest.slices.iter().skip(completed_slices as usize) {
        destination_guard.revalidate()?;
        let boundary_before = completed_bytes;
        let slice_result = (|| {
            check_cancelled(&options.cancellation)?;
            let path = directory.join(&slice.filename);
            let mut input = if let Some(package) = bound_package.as_ref() {
                package.open_slice(slice)?.into_file()
            } else {
                validate_existing_regular_file(&path)?;
                File::open(&path).map_err(|error| io_error(&path, error))?
            };
            let mut remaining = slice.size;
            let mut slice_hasher = Sha256State::new();
            while remaining > 0 {
                check_cancelled(&options.cancellation)?;
                let limit = remaining.min(buffer.len() as u64) as usize;
                let read = input
                    .read(&mut buffer[..limit])
                    .map_err(|error| io_error(&path, error))?;
                if read == 0 {
                    return Err(CoreError::CorruptedSlices(vec![slice.filename.clone()]));
                }
                output
                    .write_all(&buffer[..read])
                    .map_err(|error| io_error(&partial_path, error))?;
                slice_hasher.update(&buffer[..read]);
                remaining -= read as u64;
                completed_bytes += read as u64;
                on_progress(Progress {
                    operation: "merge",
                    bytes_processed: completed_bytes,
                    total_bytes: manifest.original.size,
                    current_slice: slice.index,
                    slice_count: manifest.slice_count,
                });
            }
            let mut probe = [0_u8; 1];
            if input
                .read(&mut probe)
                .map_err(|error| io_error(&path, error))?
                != 0
                || slice_hasher.finish() != slice.sha256
            {
                return if package_binding.is_some() {
                    Err(CoreError::PackageIdentityChanged(
                        manifest_path.to_path_buf(),
                    ))
                } else {
                    Err(CoreError::CorruptedSlices(vec![slice.filename.clone()]))
                };
            }
            output
                .flush()
                .map_err(|error| io_error(&partial_path, error))?;
            output
                .sync_all()
                .map_err(|error| io_error(&partial_path, error))?;
            Ok(())
        })();
        if let Err(error) = slice_result {
            output
                .set_len(boundary_before)
                .and_then(|_| output.sync_all())
                .map_err(|truncate_error| io_error(&partial_path, truncate_error))?;
            return Err(error);
        }
        completed_slices += 1;
        on_checkpoint(MergeCheckpointEvent::SliceBoundary {
            completed_slices,
            completed_bytes,
        });
        destination_guard.revalidate()?;
    }

    output
        .flush()
        .map_err(|error| io_error(&partial_path, error))?;
    output
        .sync_all()
        .map_err(|error| io_error(&partial_path, error))?;
    output
        .seek(SeekFrom::Start(0))
        .map_err(|error| io_error(&partial_path, error))?;
    let actual = hash_reader(&mut output, &partial_path, &options.cancellation)?;
    if actual != manifest.original.sha256 {
        return Err(CoreError::FinalHashMismatch {
            expected: manifest.original.sha256,
            actual,
        });
    }
    if let Some(package) = bound_package.as_mut() {
        package.revalidate(&options.cancellation)?;
    }
    let staged = StagedOutput {
        partial_path: partial_path.clone(),
        final_path: output_path.to_path_buf(),
        identity: FileIdentity {
            volume: partial_identity.volume,
            file: partial_identity.file,
        },
        expected_size: manifest.original.size,
        expected_sha256: manifest.original.sha256.clone(),
    };
    drop(output);
    destination_guard.revalidate()?;
    finalize_staged_output(&staged, &options.cancellation)?;
    destination_guard.revalidate()?;
    on_checkpoint(MergeCheckpointEvent::Published {
        filename: filename_of(output_path)?,
        sha256: manifest.original.sha256,
    });
    destination_guard.revalidate()?;
    Ok(())
}

pub fn fingerprint_file(path: &Path) -> Result<SourceFingerprint, CoreError> {
    validate_existing_regular_file(path)?;
    let file = File::open(path).map_err(|error| io_error(path, error))?;
    let state = source_state(&file).map_err(|error| io_error(path, error))?;
    Ok(fingerprint_source(&state))
}

pub fn fingerprint_directory(path: &Path) -> Result<DirectoryFingerprint, CoreError> {
    validate_existing_directory(path)?;
    let file = open_directory(path).map_err(|error| io_error(path, error))?;
    Ok(DirectoryFingerprint {
        identity: file_identity(&file)
            .map_err(|error| io_error(path, error))?
            .into(),
    })
}

pub fn validate_existing_regular_file(path: &Path) -> Result<(), CoreError> {
    validate_absolute_path(path)?;
    validate_no_reparse_components(path)?;
    let metadata = fs::metadata(path).map_err(|error| io_error(path, error))?;
    if !metadata.is_file() {
        return Err(CoreError::InvalidInput(path.to_path_buf()));
    }
    Ok(())
}

pub fn validate_existing_directory(path: &Path) -> Result<(), CoreError> {
    validate_absolute_path(path)?;
    validate_no_reparse_components(path)?;
    let metadata = fs::metadata(path).map_err(|error| io_error(path, error))?;
    if !metadata.is_dir() {
        return Err(CoreError::UnsafeFilesystemPath(path.to_path_buf()));
    }
    Ok(())
}

/// Removes a task-owned incomplete file only when its current native identity
/// still matches the persisted checkpoint. A missing file is already clean;
/// an identity change fails closed rather than deleting a replacement.
pub fn remove_owned_incomplete_file(
    path: &Path,
    expected: NativeFileIdentity,
) -> Result<(), CoreError> {
    validate_absolute_path(path)?;
    validate_no_reparse_components(path)?;
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(path, error)),
    };
    let actual: NativeFileIdentity = file_identity(&file)
        .map_err(|error| io_error(path, error))?
        .into();
    if actual != expected {
        return Err(CoreError::StagedIdentityChanged(path.to_path_buf()));
    }
    drop(file);
    fs::remove_file(path).map_err(|error| io_error(path, error))
}

fn validate_recovered_manifest(
    manifest_path: &Path,
    options: &ResumableSplitOptions,
    original_filename: &str,
    original_size: u64,
    baseline_sha256: &str,
    completed: &[SliceCheckpoint],
) -> Result<String, CoreError> {
    let manifest = load_manifest(manifest_path)?;
    if completed.len() as u64 != manifest.slice_count
        || manifest.package_id != options.package_id
        || manifest.created_at != options.created_at
        || manifest.original.filename != original_filename
        || manifest.original.size != original_size
        || manifest.original.sha256 != baseline_sha256
        || manifest.target_slice_size != options.slice_size
        || manifest.slices
            != completed
                .iter()
                .map(|checkpoint| checkpoint.entry.clone())
                .collect::<Vec<_>>()
    {
        return Err(CoreError::ResumeRejected(
            "the published Manifest does not match its verified checkpoint".to_owned(),
        ));
    }
    let inspection = inspect_package(manifest_path, true, &options.cancellation)?;
    if !inspection.missing.is_empty()
        || !inspection.corrupted.is_empty()
        || !inspection.unexpected.is_empty()
    {
        return Err(CoreError::ResumeRejected(
            "the published Cake Package no longer verifies".to_owned(),
        ));
    }
    let mut file = File::open(manifest_path).map_err(|error| io_error(manifest_path, error))?;
    hash_reader(&mut file, manifest_path, &options.cancellation)
}

struct SplitValidationPlan<'a> {
    output_dir: &'a Path,
    original_filename: &'a str,
    slice_size: u64,
    slice_count: u64,
    width: usize,
}

fn validate_completed_slices(
    source: &mut File,
    source_path: &Path,
    plan: &SplitValidationPlan<'_>,
    completed: &[SliceCheckpoint],
    cancellation: &CancellationToken,
) -> Result<String, CoreError> {
    if completed.len() as u64 > plan.slice_count {
        return Err(CoreError::ResumeRejected(
            "the checkpoint contains too many completed Slices".to_owned(),
        ));
    }
    let total_size = source
        .metadata()
        .map_err(|error| io_error(source_path, error))?
        .len();
    let mut stream_hasher = Sha256State::new();
    let mut buffer = vec![0_u8; DEFAULT_BUFFER_SIZE];
    for (position, checkpoint) in completed.iter().enumerate() {
        let index = position as u64 + 1;
        let (offset, size) = planned_slice_range(total_size, plan.slice_size, index)?;
        let expected_name = slice_filename(plan.original_filename, index, plan.width);
        if checkpoint.entry.index != index
            || checkpoint.entry.offset != offset
            || checkpoint.entry.size != size
            || checkpoint.entry.filename != expected_name
        {
            return Err(CoreError::ResumeRejected(
                "a completed Slice checkpoint does not match the processing plan".to_owned(),
            ));
        }
        let path = plan.output_dir.join(&expected_name);
        validate_existing_regular_file(&path)?;
        let mut output = File::open(&path).map_err(|error| io_error(&path, error))?;
        let identity: NativeFileIdentity = file_identity(&output)
            .map_err(|error| io_error(&path, error))?
            .into();
        if identity != checkpoint.identity
            || output
                .metadata()
                .map_err(|error| io_error(&path, error))?
                .len()
                != size
        {
            return Err(CoreError::ResumeRejected(format!(
                "completed Slice {index} changed"
            )));
        }
        let mut slice_hasher = Sha256State::new();
        let mut remaining = size;
        while remaining > 0 {
            check_cancelled(cancellation)?;
            let limit = remaining.min(buffer.len() as u64) as usize;
            let read = output
                .read(&mut buffer[..limit])
                .map_err(|error| io_error(&path, error))?;
            if read == 0 {
                return Err(CoreError::ResumeRejected(format!(
                    "completed Slice {index} changed"
                )));
            }
            slice_hasher.update(&buffer[..read]);
            stream_hasher.update(&buffer[..read]);
            remaining -= read as u64;
        }
        let mut probe = [0_u8; 1];
        if output
            .read(&mut probe)
            .map_err(|error| io_error(&path, error))?
            != 0
            || slice_hasher.finish() != checkpoint.entry.sha256
            || NativeFileIdentity::from(
                file_identity(&output).map_err(|error| io_error(&path, error))?,
            ) != checkpoint.identity
        {
            return Err(CoreError::ResumeRejected(format!(
                "completed Slice {index} changed"
            )));
        }
        if hash_range(source, source_path, offset, size, cancellation)? != checkpoint.entry.sha256 {
            return Err(CoreError::ResumeRejected(format!(
                "source range for completed Slice {index} changed"
            )));
        }
    }
    Ok(stream_hasher.finish())
}

fn verify_partial_prefix(
    partial: &mut File,
    partial_path: &Path,
    slices: &[SliceEntry],
    completed_slices: u64,
    cancellation: &CancellationToken,
) -> Result<(), CoreError> {
    partial
        .seek(SeekFrom::Start(0))
        .map_err(|error| io_error(partial_path, error))?;
    let mut buffer = vec![0_u8; DEFAULT_BUFFER_SIZE];
    for slice in slices.iter().take(completed_slices as usize) {
        let mut remaining = slice.size;
        let mut hasher = Sha256State::new();
        while remaining > 0 {
            check_cancelled(cancellation)?;
            let limit = remaining.min(buffer.len() as u64) as usize;
            let read = partial
                .read(&mut buffer[..limit])
                .map_err(|error| io_error(partial_path, error))?;
            if read == 0 {
                return Err(CoreError::ResumeRejected(
                    "the partial-output prefix is shorter than its checkpoint".to_owned(),
                ));
            }
            hasher.update(&buffer[..read]);
            remaining -= read as u64;
        }
        if hasher.finish() != slice.sha256 {
            return Err(CoreError::ResumeRejected(format!(
                "partial-output Slice {} failed verification",
                slice.index
            )));
        }
    }
    Ok(())
}

fn hash_range(
    file: &mut File,
    path: &Path,
    offset: u64,
    size: u64,
    cancellation: &CancellationToken,
) -> Result<String, CoreError> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| io_error(path, error))?;
    let mut remaining = size;
    let mut buffer = vec![0_u8; DEFAULT_BUFFER_SIZE];
    let mut hasher = Sha256State::new();
    while remaining > 0 {
        check_cancelled(cancellation)?;
        let limit = remaining.min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..limit])
            .map_err(|error| io_error(path, error))?;
        if read == 0 {
            return Err(CoreError::SourceChanged);
        }
        hasher.update(&buffer[..read]);
        remaining -= read as u64;
    }
    Ok(hasher.finish())
}

fn remove_owned_partial(path: &Path, expected: NativeFileIdentity) -> Result<(), CoreError> {
    let file = File::open(path).map_err(|error| io_error(path, error))?;
    let actual: NativeFileIdentity = file_identity(&file)
        .map_err(|error| io_error(path, error))?
        .into();
    if actual != expected {
        return Err(CoreError::StagedIdentityChanged(path.to_path_buf()));
    }
    drop(file);
    fs::remove_file(path).map_err(|error| io_error(path, error))
}

fn fingerprint_source(state: &SourceState) -> SourceFingerprint {
    let nanos = state
        .modified
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0);
    SourceFingerprint {
        identity: state.identity.into(),
        len: state.len,
        modified_unix_nanos: nanos,
    }
}

fn task_partial_path(final_path: &Path, task_id: &str) -> Result<PathBuf, CoreError> {
    validate_task_id(task_id)?;
    let filename = filename_of(final_path)?;
    Ok(final_path.with_file_name(format!("{filename}.{task_id}.partial")))
}

fn filename_of(path: &Path) -> Result<String, CoreError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .ok_or(CoreError::NonUtf8Filename)
}

fn validate_task_id(value: &str) -> Result<(), CoreError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| CoreError::ResumeRejected("task ID is invalid".to_owned()))
}

fn validate_package_id(value: &str) -> Result<(), CoreError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| CoreError::ResumeRejected("package ID is invalid".to_owned()))
}

fn validate_absolute_path(path: &Path) -> Result<(), CoreError> {
    if !path.is_absolute() {
        return Err(CoreError::UnsafeFilesystemPath(path.to_path_buf()));
    }
    Ok(())
}

pub(crate) fn validate_no_reparse_components(path: &Path) -> Result<(), CoreError> {
    for component in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        let metadata = match fs::symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_error(component, error)),
        };
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(CoreError::UnsafeFilesystemPath(component.to_path_buf()));
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(crate) fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
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

pub fn default_created_at() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
