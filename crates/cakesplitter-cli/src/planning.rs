use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use cakesplitter_core::{
    CancellationToken, PackageBinding, PackageInspection, capture_package_binding,
    find_package_manifest, inspect_package_bound, load_manifest, validate_existing_directory,
    validate_existing_regular_file,
};
use cakesplitter_format::{
    CakeManifest, FORMAT_VERSION, MAX_FILENAME_BYTES, MAX_MANIFEST_BYTES, MAX_SAFE_INTEGER,
    MAX_SLICE_COUNT, expected_slice_count, slice_filename, slice_index_width,
    validate_portable_filename,
};
use serde_json::{Value, json};

use crate::{
    cli::{MergePlanArgs, PackageArgs, SplitPlanArgs},
    error::{CliError, CliErrorCategory, EXIT_CAPACITY, EXIT_CONFLICT, EXIT_SOURCE},
};

const RECOMMENDED_HEADROOM_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug)]
pub struct SplitPrepared {
    pub source: PathBuf,
    pub output_directory: PathBuf,
    pub slice_size: u64,
    pub expected_slice_count: u64,
    pub plan: Value,
    pub ready: bool,
    pub conflicts: Vec<String>,
    pub insufficient_space: bool,
}

#[derive(Debug)]
pub struct PackagePrepared {
    pub manifest_path: PathBuf,
    pub binding: PackageBinding,
    pub inspection: PackageInspection,
}

#[derive(Debug)]
pub struct MergePrepared {
    pub package: PackagePrepared,
    pub output: PathBuf,
    pub plan: Value,
    pub ready: bool,
    pub conflicts: Vec<String>,
    pub insufficient_space: bool,
}

pub fn plan_split(arguments: &SplitPlanArgs) -> Result<SplitPrepared, CliError> {
    let source = canonical_existing_file(&arguments.file)?;
    let metadata = fs::metadata(&source).map_err(core_io)?;
    if !metadata.is_file() {
        return Err(source_error(
            "source_not_regular",
            "The selected source is not a regular file.",
        ));
    }
    let source_size = metadata.len();
    if source_size > MAX_SAFE_INTEGER {
        return Err(capacity_error(
            "source_size_limit",
            "The selected source exceeds the cross-runtime safe integer limit.",
        ));
    }
    let source_filename = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            source_error(
                "non_utf8_filename",
                "The source filename is not valid UTF-8.",
            )
        })?;
    validate_portable_filename(source_filename)
        .map_err(|error| CliError::from(cakesplitter_core::CoreError::from(error)))?;

    let slice_size = match (arguments.slice_size, arguments.slice_count) {
        (Some(size), None) => size,
        (None, Some(count)) => slice_size_for_count(source_size, count)?,
        _ => {
            return Err(CliError::usage(
                "invalid_split_plan",
                "Choose exactly one target: --slice-size or --slice-count.",
                "split target options were missing or conflicting",
            ));
        }
    };
    let slice_count = expected_slice_count(source_size, slice_size);
    if slice_count > MAX_SLICE_COUNT {
        return Err(capacity_error(
            "slice_count_limit",
            format!("The plan requires {slice_count} Slices; the maximum is {MAX_SLICE_COUNT}."),
        ));
    }

    let output_directory = absolute_path(
        arguments
            .output_dir
            .as_deref()
            .unwrap_or_else(|| source.parent().unwrap_or_else(|| Path::new("."))),
    )?;
    validate_destination_ancestors(&output_directory)?;
    let width = slice_index_width(slice_count);
    let mut expected_outputs = Vec::with_capacity(slice_count as usize + 1);
    for index in 1..=slice_count {
        expected_outputs.push(slice_filename(source_filename, index, width));
    }
    expected_outputs.push(format!("{source_filename}.cake.json"));
    let conflicts = output_conflicts(&output_directory, &expected_outputs);
    let available_free_space = available_space_for(&output_directory).unwrap_or(0);
    let required_free_space = source_size;
    let recommended_free_space = required_free_space.saturating_add(RECOMMENDED_HEADROOM_BYTES);
    let insufficient_space = available_free_space < required_free_space;
    let mut warnings = Vec::new();
    if !output_directory.exists() {
        warnings.push("output-directory-will-be-created".to_owned());
    }
    if available_free_space < recommended_free_space && !insufficient_space {
        warnings.push("free-space-below-recommended".to_owned());
    }
    if insufficient_space {
        warnings.push("insufficient-free-space".to_owned());
    }
    let ready = conflicts.is_empty() && !insufficient_space;
    let plan = json!({
        "type": "split",
        "sourceFilename": source_filename,
        "sourceSize": source_size,
        "targetSliceSize": slice_size,
        "expectedSliceCount": slice_count,
        "expectedOutputNames": expected_outputs,
        "requiredFreeSpace": required_free_space,
        "availableFreeSpace": available_free_space,
        "warnings": warnings,
        "conflicts": conflicts,
        "compatibilityLimits": compatibility_limits(),
        "cakePackageFormat": FORMAT_VERSION,
        "ready": ready
    });
    Ok(SplitPrepared {
        source,
        output_directory,
        slice_size,
        expected_slice_count: slice_count,
        plan,
        ready,
        conflicts,
        insufficient_space,
    })
}

pub fn prepare_package(
    arguments: &PackageArgs,
    verify_hashes: bool,
    cancellation: &CancellationToken,
) -> Result<PackagePrepared, CliError> {
    let manifest_path = resolve_manifest(&arguments.package, cancellation)?;
    validate_explicit_slices(&manifest_path, &arguments.slices, cancellation)?;
    let binding = capture_package_binding(&manifest_path, cancellation).map_err(CliError::from)?;
    let inspection = inspect_package_bound(&manifest_path, verify_hashes, &binding, cancellation)
        .map_err(CliError::from)?;
    Ok(PackagePrepared {
        manifest_path,
        binding,
        inspection,
    })
}

pub fn plan_merge(
    arguments: &MergePlanArgs,
    cancellation: &CancellationToken,
) -> Result<MergePrepared, CliError> {
    let package_arguments = PackageArgs {
        package: arguments.package.clone(),
        slices: arguments.slices.clone(),
    };
    let package = prepare_package(&package_arguments, true, cancellation)?;
    let output = absolute_path(&arguments.output)?;
    validate_output_filename(&output)?;
    if let Some(parent) = output.parent() {
        validate_destination_ancestors(parent)?;
    }
    let manifest = &package.inspection.manifest;
    let conflicts = output_file_conflicts(&output);
    let available_free_space =
        available_space_for(output.parent().unwrap_or_else(|| Path::new("."))).unwrap_or(0);
    let required_free_space = manifest.original.size;
    let recommended_free_space = required_free_space.saturating_add(RECOMMENDED_HEADROOM_BYTES);
    let insufficient_space = available_free_space < required_free_space;
    let integrity_ready = package.inspection.missing.is_empty()
        && package.inspection.corrupted.is_empty()
        && package.inspection.unexpected.is_empty();
    let mut warnings = Vec::new();
    if available_free_space < recommended_free_space && !insufficient_space {
        warnings.push("free-space-below-recommended".to_owned());
    }
    if insufficient_space {
        warnings.push("insufficient-free-space".to_owned());
    }
    let ready = integrity_ready && conflicts.is_empty() && !insufficient_space;
    let plan = json!({
        "type": "merge",
        "manifestFilename": package.manifest_path.file_name().and_then(|name| name.to_str()).unwrap_or("<manifest>"),
        "originalFilename": manifest.original.filename,
        "sourceSize": manifest.original.size,
        "targetSliceSize": manifest.target_slice_size,
        "expectedSliceCount": manifest.slice_count,
        "expectedOutputNames": [output.file_name().and_then(|name| name.to_str()).unwrap_or("<output>")],
        "requiredFreeSpace": required_free_space,
        "availableFreeSpace": available_free_space,
        "warnings": warnings,
        "conflicts": conflicts,
        "missing": package.inspection.missing,
        "corrupted": package.inspection.corrupted,
        "duplicateSlices": [],
        "unexpected": package.inspection.unexpected,
        "hashesVerified": package.inspection.verified,
        "compatibilityLimits": compatibility_limits(),
        "cakePackageFormat": FORMAT_VERSION,
        "ready": ready
    });
    Ok(MergePrepared {
        package,
        output,
        plan,
        ready,
        conflicts,
        insufficient_space,
    })
}

pub fn manifest_only(
    package: &Path,
    cancellation: &CancellationToken,
) -> Result<(PathBuf, CakeManifest), CliError> {
    let manifest_path = resolve_manifest(package, cancellation)?;
    let manifest = load_manifest(&manifest_path).map_err(CliError::from)?;
    Ok((manifest_path, manifest))
}

pub fn ensure_split_ready(plan: &SplitPrepared) -> Result<(), CliError> {
    if plan.ready {
        return Ok(());
    }
    if !plan.conflicts.is_empty() {
        return Err(conflict_error(&plan.conflicts));
    }
    if plan.insufficient_space {
        return Err(capacity_error(
            "insufficient_space",
            "The destination does not have enough available free space.",
        ));
    }
    Err(CliError::internal(
        "split plan was not ready without a documented cause",
    ))
}

pub fn ensure_merge_ready(plan: &MergePrepared) -> Result<(), CliError> {
    if plan.ready {
        return Ok(());
    }
    if !plan.conflicts.is_empty() {
        return Err(conflict_error(&plan.conflicts));
    }
    if plan.insufficient_space {
        return Err(capacity_error(
            "insufficient_space",
            "The destination does not have enough available free space.",
        ));
    }
    if !plan.package.inspection.missing.is_empty() {
        return Err(CliError::from(cakesplitter_core::CoreError::MissingSlices(
            plan.package.inspection.missing.clone(),
        )));
    }
    if !plan.package.inspection.corrupted.is_empty() {
        return Err(CliError::from(
            cakesplitter_core::CoreError::CorruptedSlices(
                plan.package.inspection.corrupted.clone(),
            ),
        ));
    }
    if !plan.package.inspection.unexpected.is_empty() {
        return Err(CliError::from(
            cakesplitter_core::CoreError::UnexpectedSlices(
                plan.package.inspection.unexpected.clone(),
            ),
        ));
    }
    Err(CliError::internal(
        "merge plan was not ready without a documented cause",
    ))
}

pub fn absolute_path(path: &Path) -> Result<PathBuf, CliError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| CliError::internal(error.to_string()))
}

pub fn canonical_existing_file(path: &Path) -> Result<PathBuf, CliError> {
    let absolute = absolute_path(path)?;
    validate_existing_regular_file(&absolute).map_err(|error| match error {
        cakesplitter_core::CoreError::UnsafeFilesystemPath(_) => source_error(
            "unsafe_filesystem_path",
            "The selected file path contains a link or reparse point and was rejected.",
        ),
        other => CliError::from(other),
    })?;
    Ok(absolute)
}

pub fn canonical_existing_directory(path: &Path) -> Result<PathBuf, CliError> {
    let absolute = absolute_path(path)?;
    validate_existing_directory(&absolute).map_err(CliError::from)?;
    Ok(absolute)
}

fn resolve_manifest(package: &Path, cancellation: &CancellationToken) -> Result<PathBuf, CliError> {
    let package = absolute_path(package)?;
    let path = if package.is_dir() {
        let directory = canonical_existing_directory(&package)?;
        find_package_manifest(&directory, cancellation).map_err(CliError::from)?
    } else {
        canonical_existing_file(&package)?
    };
    canonical_existing_file(&path)
}

fn validate_explicit_slices(
    manifest_path: &Path,
    selected: &[PathBuf],
    cancellation: &CancellationToken,
) -> Result<(), CliError> {
    if selected.is_empty() {
        return Ok(());
    }
    let manifest = load_manifest(manifest_path).map_err(CliError::from)?;
    if selected.len() != manifest.slices.len() {
        return Err(CliError::usage(
            "selected_slice_count_mismatch",
            "Explicit Slice selection must contain the complete expected set.",
            format!(
                "expected {} selected Slices, received {}",
                manifest.slices.len(),
                selected.len()
            ),
        ));
    }
    let package_directory = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let expected = manifest
        .slices
        .iter()
        .map(|slice| slice.filename.as_str())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    for selected_path in selected {
        if cancellation.is_cancelled() {
            return Err(CliError::from(cakesplitter_core::CoreError::Cancelled));
        }
        let selected_path = canonical_existing_file(selected_path)?;
        if selected_path.parent() != Some(package_directory) {
            return Err(CliError::usage(
                "selected_slice_outside_package",
                "Explicit Slices must belong to the selected Manifest package directory.",
                "an explicitly selected Slice was outside the bound package directory",
            ));
        }
        let name = selected_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                source_error(
                    "non_utf8_filename",
                    "A selected Slice filename is not valid UTF-8.",
                )
            })?;
        if !expected.contains(name) || !seen.insert(name.to_owned()) {
            return Err(CliError::usage(
                "invalid_selected_slice_set",
                "Explicit Slice selection contains a duplicate or unexpected file.",
                "explicit selected-set membership did not match the Manifest",
            ));
        }
    }
    Ok(())
}

fn slice_size_for_count(source_size: u64, count: u64) -> Result<u64, CliError> {
    if source_size == 0 {
        return Err(CliError::usage(
            "slice_count_for_empty_source",
            "An empty source has zero Slices; use --slice-size instead.",
            "target Slice count cannot be applied to an empty source",
        ));
    }
    if count > source_size {
        return Err(CliError::usage(
            "slice_count_exceeds_source_bytes",
            "The requested Slice count exceeds one Slice per source byte.",
            format!("requested {count} Slices for {source_size} bytes"),
        ));
    }
    let slice_size = source_size.div_ceil(count);
    if expected_slice_count(source_size, slice_size) != count {
        return Err(CliError::usage(
            "slice_count_not_representable",
            "The requested Slice count cannot be represented by one uniform target Slice size.",
            format!(
                "{source_size} bytes cannot produce exactly {count} Slices with a fixed target size"
            ),
        ));
    }
    Ok(slice_size)
}

fn output_conflicts(output_directory: &Path, names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|name| {
            let final_path = output_directory.join(name);
            if fs::symlink_metadata(&final_path).is_ok() {
                return true;
            }
            let prefix = format!("{name}.");
            fs::read_dir(output_directory)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .any(|entry| {
                    entry.file_name().to_str().is_some_and(|candidate| {
                        candidate.starts_with(&prefix) && candidate.ends_with(".partial")
                    })
                })
        })
        .cloned()
        .collect()
}

fn output_file_conflicts(output: &Path) -> Vec<String> {
    let task_partial_prefix = output
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}."));
    let mut conflicts = Vec::new();
    if fs::symlink_metadata(output).is_ok() {
        conflicts.push(
            output
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("<output>")
                .to_owned(),
        );
    }
    if let (Some(parent), Some(prefix)) = (output.parent(), task_partial_prefix)
        && let Ok(entries) = fs::read_dir(parent)
        && entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".partial"))
        })
    {
        conflicts.push("task-owned partial output".to_owned());
    }
    conflicts
}

fn available_space_for(path: &Path) -> std::io::Result<u64> {
    let mut candidate = path;
    while !candidate.exists() {
        candidate = candidate.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no existing destination ancestor",
            )
        })?;
    }
    fs4::available_space(candidate)
}

fn validate_destination_ancestors(path: &Path) -> Result<(), CliError> {
    let mut candidate = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&candidate) {
            Ok(_) => return validate_existing_directory(&candidate).map_err(CliError::from),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                candidate = candidate
                    .parent()
                    .ok_or_else(|| {
                        CliError::from(cakesplitter_core::CoreError::UnsafeFilesystemPath(
                            path.to_path_buf(),
                        ))
                    })?
                    .to_path_buf();
            }
            Err(error) => {
                return Err(CliError::from(cakesplitter_core::CoreError::Io {
                    path: candidate,
                    source: error,
                }));
            }
        }
    }
}

fn validate_output_filename(path: &Path) -> Result<(), CliError> {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            source_error(
                "non_utf8_filename",
                "The output filename is not valid UTF-8.",
            )
        })?;
    validate_portable_filename(filename)
        .map_err(|error| CliError::from(cakesplitter_core::CoreError::from(error)))
}

fn compatibility_limits() -> Value {
    json!({
        "maximumSliceCount": MAX_SLICE_COUNT,
        "maximumManifestBytes": MAX_MANIFEST_BYTES,
        "maximumFilenameBytes": MAX_FILENAME_BYTES,
        "maximumSafeInteger": MAX_SAFE_INTEGER
    })
}

fn conflict_error(conflicts: &[String]) -> CliError {
    CliError {
        code: "output_collision".to_owned(),
        category: CliErrorCategory::Conflict,
        message: "One or more planned outputs already exist; nothing was overwritten.".to_owned(),
        technical_message: format!("{} output conflict(s) detected", conflicts.len()),
        retryable: true,
        suggested_action: "Choose a new output path or remove conflicts explicitly.".to_owned(),
        operation_id: None,
        exit_code: EXIT_CONFLICT,
    }
}

fn source_error(code: &str, message: impl Into<String>) -> CliError {
    let message = message.into();
    CliError {
        code: code.to_owned(),
        category: CliErrorCategory::Source,
        message: message.clone(),
        technical_message: message,
        retryable: false,
        suggested_action: "Select a stable local source and retry.".to_owned(),
        operation_id: None,
        exit_code: EXIT_SOURCE,
    }
}

fn capacity_error(code: &str, message: impl Into<String>) -> CliError {
    let message = message.into();
    CliError {
        code: code.to_owned(),
        category: CliErrorCategory::Capacity,
        message: message.clone(),
        technical_message: message,
        retryable: false,
        suggested_action: "Reduce the operation size or choose a destination with more space."
            .to_owned(),
        operation_id: None,
        exit_code: EXIT_CAPACITY,
    }
}

fn core_io(error: std::io::Error) -> CliError {
    CliError::from(cakesplitter_core::CoreError::Io {
        path: PathBuf::from("<selected-source>"),
        source: error,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn split_plan_is_read_only_and_reports_expected_outputs() {
        let root = tempdir().unwrap();
        let source = root.path().join("hello world.bin");
        let output = root.path().join("future package");
        fs::write(&source, vec![1_u8; 9]).unwrap();
        let before = fs::read_dir(root.path()).unwrap().count();
        let plan = plan_split(&SplitPlanArgs {
            file: source,
            slice_size: Some(4),
            slice_count: None,
            output_dir: Some(output.clone()),
        })
        .unwrap();
        assert_eq!(plan.expected_slice_count, 3);
        assert!(plan.ready);
        assert!(!output.exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), before);
        assert_eq!(
            plan.plan["expectedOutputNames"].as_array().unwrap().len(),
            4
        );
    }

    #[test]
    fn target_count_is_deterministic_and_bounded() {
        assert_eq!(slice_size_for_count(10, 3).unwrap(), 4);
        assert!(slice_size_for_count(2, 3).is_err());
        assert!(slice_size_for_count(0, 1).is_err());
    }
}
