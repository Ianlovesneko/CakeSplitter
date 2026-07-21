use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use cakesplitter_core::{
    CancellationToken, CoreError, DirectoryFingerprint, SourceFingerprint, find_package_manifest,
    fingerprint_directory, fingerprint_file, validate_existing_directory,
    validate_existing_regular_file,
};
use cakesplitter_format::{MAX_SLICE_COUNT, validate_portable_filename};
use serde::Serialize;
use uuid::Uuid;

use crate::CommandError;

const MAX_SELECTIONS: usize = 64;
const SELECTION_LIFETIME: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectionKind {
    SourceFile,
    ManifestFile,
    PackageFolder,
    OutputFolder,
    OutputFile,
    SliceFiles,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionSummary {
    pub token: String,
    pub kind: SelectionKind,
    pub display_name: String,
    pub size: Option<u64>,
    pub count: u64,
}

#[derive(Clone, Debug)]
pub struct ResolvedOutputFile {
    pub path: PathBuf,
    pub parent: PathBuf,
    pub parent_identity: DirectoryFingerprint,
}

#[derive(Clone, Debug)]
pub struct ResolvedOutputDirectory {
    pub path: PathBuf,
    pub identity: DirectoryFingerprint,
}

#[derive(Clone, Debug)]
enum SelectionValue {
    One(PathBuf),
    Many(Vec<PathBuf>),
}

#[derive(Clone, Debug)]
enum SelectionBinding {
    File(SourceFingerprint),
    Directory(DirectoryFingerprint),
    FutureOutput {
        parent: PathBuf,
        parent_identity: DirectoryFingerprint,
    },
    ManyFiles {
        files: Vec<SourceFingerprint>,
        parent: PathBuf,
        parent_identity: DirectoryFingerprint,
    },
}

#[derive(Clone, Debug)]
struct SelectionEntry {
    kind: SelectionKind,
    value: SelectionValue,
    binding: SelectionBinding,
    created: Instant,
}

#[derive(Default)]
pub struct SelectionRegistry {
    entries: Mutex<HashMap<String, SelectionEntry>>,
}

impl SelectionRegistry {
    pub fn issue_file(
        &self,
        path: PathBuf,
        kind: SelectionKind,
    ) -> Result<SelectionSummary, CommandError> {
        validate_existing_regular_file(&path)?;
        match kind {
            SelectionKind::SourceFile => {}
            SelectionKind::ManifestFile => ensure_extension(&path, ".cake.json")?,
            _ => {
                return Err(CommandError::new(
                    "invalid_selection",
                    "Invalid file selection.",
                ));
            }
        }
        let binding = fingerprint_file(&path)?;
        let size = binding.len;
        let display_name = filename(&path)?;
        self.issue(
            kind,
            SelectionValue::One(path),
            SelectionBinding::File(binding),
            display_name,
            Some(size),
            1,
        )
    }

    pub fn issue_directory(
        &self,
        path: PathBuf,
        kind: SelectionKind,
    ) -> Result<SelectionSummary, CommandError> {
        validate_existing_directory(&path)?;
        if !matches!(
            kind,
            SelectionKind::PackageFolder | SelectionKind::OutputFolder
        ) {
            return Err(CommandError::new(
                "invalid_selection",
                "Invalid folder selection.",
            ));
        }
        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Selected folder")
            .to_owned();
        let binding = fingerprint_directory(&path)?;
        self.issue(
            kind,
            SelectionValue::One(path),
            SelectionBinding::Directory(binding),
            display_name,
            None,
            1,
        )
    }

    pub fn issue_output_file(&self, path: PathBuf) -> Result<SelectionSummary, CommandError> {
        if !path.is_absolute() {
            return Err(CommandError::new(
                "unsafe_filesystem_path",
                "The selected output path is not absolute.",
            ));
        }
        let parent = path.parent().ok_or_else(|| {
            CommandError::new("unsafe_filesystem_path", "The output folder is invalid.")
        })?;
        validate_existing_directory(parent)?;
        if !path_is_absent(&path)? {
            return Err(CommandError::new(
                "output_collision",
                "The selected output file already exists.",
            ));
        }
        let display_name = filename(&path)?;
        validate_portable_filename(&display_name).map_err(CoreError::from)?;
        let parent = parent.to_path_buf();
        let parent_identity = fingerprint_directory(&parent)?;
        self.issue(
            SelectionKind::OutputFile,
            SelectionValue::One(path),
            SelectionBinding::FutureOutput {
                parent,
                parent_identity,
            },
            display_name,
            None,
            1,
        )
    }

    pub fn issue_slices(&self, paths: Vec<PathBuf>) -> Result<SelectionSummary, CommandError> {
        if paths.is_empty() || paths.len() > MAX_SLICE_COUNT as usize {
            return Err(CommandError::new(
                "resource_limit",
                "Select between 1 and 50,000 Slice files.",
            ));
        }
        let mut total = 0_u64;
        let mut unique = HashSet::with_capacity(paths.len());
        let mut unique_identities = HashSet::with_capacity(paths.len());
        let parent = paths
            .first()
            .and_then(|path| path.parent())
            .ok_or_else(|| CommandError::new("invalid_selection", "Slice folder is invalid."))?
            .to_path_buf();
        let parent_identity = fingerprint_directory(&parent)?;
        let mut bindings = Vec::with_capacity(paths.len());
        for path in &paths {
            if !unique.insert(path.clone()) {
                return Err(CommandError::new(
                    "duplicate_slice",
                    "The Slice selection contains a duplicate file.",
                ));
            }
            validate_existing_regular_file(path)?;
            ensure_extension(path, ".slice")?;
            if path.parent() != Some(parent.as_path()) {
                return Err(CommandError::new(
                    "package_match_ambiguous",
                    "Selected Slices must come from one Cake Package folder.",
                ));
            }
            let binding = fingerprint_file(path)?;
            if !unique_identities.insert((binding.identity.volume, binding.identity.file)) {
                return Err(CommandError::new(
                    "duplicate_slice",
                    "The Slice selection contains the same file more than once.",
                ));
            }
            total = total
                .checked_add(binding.len)
                .ok_or_else(|| CommandError::new("resource_limit", "Selected size overflowed."))?;
            bindings.push(binding);
        }
        let count = paths.len() as u64;
        self.issue(
            SelectionKind::SliceFiles,
            SelectionValue::Many(paths),
            SelectionBinding::ManyFiles {
                files: bindings,
                parent,
                parent_identity,
            },
            format!("{count} selected Slices"),
            Some(total),
            count,
        )
    }

    pub fn resolve_one(
        &self,
        token: &str,
        allowed: &[SelectionKind],
    ) -> Result<PathBuf, CommandError> {
        validate_token(token)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        prune(&mut entries);
        let entry = entries.get(token).ok_or_else(expired_selection)?;
        if !allowed.contains(&entry.kind) {
            return Err(CommandError::new(
                "selection_type_mismatch",
                "The selection cannot be used for this operation.",
            ));
        }
        revalidate_entry(entry)?;
        match &entry.value {
            SelectionValue::One(path) => Ok(path.clone()),
            SelectionValue::Many(_) => Err(CommandError::new(
                "selection_type_mismatch",
                "A single selected path is required.",
            )),
        }
    }

    pub fn resolve_output_file(&self, token: &str) -> Result<ResolvedOutputFile, CommandError> {
        validate_token(token)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        prune(&mut entries);
        let entry = entries.get(token).ok_or_else(expired_selection)?;
        if entry.kind != SelectionKind::OutputFile {
            return Err(CommandError::new(
                "selection_type_mismatch",
                "The selection cannot be used for this operation.",
            ));
        }
        revalidate_entry(entry)?;
        match (&entry.value, &entry.binding) {
            (
                SelectionValue::One(path),
                SelectionBinding::FutureOutput {
                    parent,
                    parent_identity,
                },
            ) => Ok(ResolvedOutputFile {
                path: path.clone(),
                parent: parent.clone(),
                parent_identity: parent_identity.clone(),
            }),
            _ => Err(CommandError::new(
                "selection_identity_changed",
                "The selected output could not be verified.",
            )),
        }
    }

    pub fn resolve_output_directory(
        &self,
        token: &str,
    ) -> Result<ResolvedOutputDirectory, CommandError> {
        validate_token(token)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        prune(&mut entries);
        let entry = entries.get(token).ok_or_else(expired_selection)?;
        if entry.kind != SelectionKind::OutputFolder {
            return Err(CommandError::new(
                "selection_type_mismatch",
                "The selection cannot be used for this operation.",
            ));
        }
        revalidate_entry(entry)?;
        match (&entry.value, &entry.binding) {
            (SelectionValue::One(path), SelectionBinding::Directory(identity)) => {
                Ok(ResolvedOutputDirectory {
                    path: path.clone(),
                    identity: identity.clone(),
                })
            }
            _ => Err(CommandError::new(
                "selection_identity_changed",
                "The selected output folder could not be verified.",
            )),
        }
    }

    fn resolve_package_source(&self, token: &str) -> Result<PathBuf, CommandError> {
        validate_token(token)?;
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        prune(&mut entries);
        let entry = entries.get(token).ok_or_else(expired_selection)?;
        revalidate_entry(entry)?;
        match (&entry.kind, &entry.value) {
            (
                SelectionKind::ManifestFile | SelectionKind::PackageFolder,
                SelectionValue::One(path),
            ) => Ok(path.clone()),
            (SelectionKind::SliceFiles, SelectionValue::Many(paths)) => {
                let parent = paths
                    .first()
                    .and_then(|path| path.parent())
                    .ok_or_else(|| {
                        CommandError::new("invalid_selection", "Slice folder is invalid.")
                    })?;
                if paths.iter().any(|path| path.parent() != Some(parent)) {
                    return Err(CommandError::new(
                        "package_match_ambiguous",
                        "Selected Slices must come from one Cake Package folder.",
                    ));
                }
                Ok(parent.to_path_buf())
            }
            _ => Err(CommandError::new(
                "selection_type_mismatch",
                "The selection cannot be used for this operation.",
            )),
        }
    }

    fn issue(
        &self,
        kind: SelectionKind,
        value: SelectionValue,
        binding: SelectionBinding,
        display_name: String,
        size: Option<u64>,
        count: u64,
    ) -> Result<SelectionSummary, CommandError> {
        let token = Uuid::new_v4().to_string();
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        prune(&mut entries);
        if entries.len() >= MAX_SELECTIONS {
            let oldest = entries
                .iter()
                .min_by_key(|(_, entry)| entry.created)
                .map(|(token, _)| token.clone());
            if let Some(oldest) = oldest {
                entries.remove(&oldest);
            }
        }
        entries.insert(
            token.clone(),
            SelectionEntry {
                kind,
                value,
                binding,
                created: Instant::now(),
            },
        );
        Ok(SelectionSummary {
            token,
            kind,
            display_name,
            size,
            count,
        })
    }
}

pub fn manifest_from_selection(
    registry: &SelectionRegistry,
    token: &str,
) -> Result<PathBuf, CommandError> {
    let path = registry.resolve_package_source(token)?;
    if path.is_file() {
        return Ok(path);
    }
    match find_package_manifest(&path, &CancellationToken::new()) {
        Ok(manifest) => Ok(manifest),
        Err(CoreError::ResumeRejected(_)) => Err(CommandError::new(
            "package_match_ambiguous",
            "The selected folder must contain exactly one Cake Manifest.",
        )),
        Err(error) => Err(error.into()),
    }
}

fn revalidate_entry(entry: &SelectionEntry) -> Result<(), CommandError> {
    let matches = match (&entry.value, &entry.binding) {
        (SelectionValue::One(path), SelectionBinding::File(expected)) => {
            fingerprint_file(path).is_ok_and(|actual| actual == *expected)
        }
        (SelectionValue::One(path), SelectionBinding::Directory(expected)) => {
            fingerprint_directory(path).is_ok_and(|actual| actual == *expected)
        }
        (
            SelectionValue::One(path),
            SelectionBinding::FutureOutput {
                parent,
                parent_identity,
            },
        ) => {
            path_is_absent(path).unwrap_or(false)
                && path.parent() == Some(parent.as_path())
                && fingerprint_directory(parent).is_ok_and(|actual| actual == *parent_identity)
        }
        (
            SelectionValue::Many(paths),
            SelectionBinding::ManyFiles {
                files,
                parent,
                parent_identity,
            },
        ) => {
            paths.len() == files.len()
                && fingerprint_directory(parent).is_ok_and(|actual| actual == *parent_identity)
                && paths.iter().zip(files).all(|(path, expected)| {
                    path.parent() == Some(parent.as_path())
                        && fingerprint_file(path).is_ok_and(|actual| actual == *expected)
                })
        }
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(CommandError::new(
            "selection_identity_changed",
            "The selected local object changed or its identity could not be verified. Select it again.",
        ))
    }
}

fn validate_token(token: &str) -> Result<(), CommandError> {
    Uuid::parse_str(token)
        .map(|_| ())
        .map_err(|_| CommandError::new("invalid_selection_token", "Selection token is invalid."))
}

fn path_is_absent(path: &Path) -> Result<bool, CommandError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err(CommandError::new(
            "selection_identity_changed",
            "The selected local object could not be verified. Select it again.",
        )),
    }
}

fn prune(entries: &mut HashMap<String, SelectionEntry>) {
    entries.retain(|_, entry| entry.created.elapsed() <= SELECTION_LIFETIME);
}

fn expired_selection() -> CommandError {
    CommandError::new(
        "selection_expired",
        "This local selection expired. Select it again.",
    )
}

fn ensure_extension(path: &Path, suffix: &str) -> Result<(), CommandError> {
    let name = filename(path)?;
    if !name.to_ascii_lowercase().ends_with(suffix) {
        return Err(CommandError::new(
            "invalid_selection",
            format!("Select a {suffix} file."),
        ));
    }
    Ok(())
}

fn filename(path: &Path) -> Result<String, CommandError> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| CommandError::new("invalid_filename", "The filename is not valid UTF-8."))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn rejects_invalid_tokens_kind_confusion_duplicates_and_existing_outputs() {
        let root = tempdir().unwrap();
        let registry = SelectionRegistry::default();
        let invalid = registry
            .resolve_one("not-a-uuid", &[SelectionKind::SourceFile])
            .unwrap_err();
        assert_eq!(invalid.code, "invalid_selection_token");

        let folder = registry
            .issue_directory(root.path().to_path_buf(), SelectionKind::OutputFolder)
            .unwrap();
        let mismatch = manifest_from_selection(&registry, &folder.token).unwrap_err();
        assert_eq!(mismatch.code, "selection_type_mismatch");

        let slice = root.path().join("sample.bin.001.slice");
        fs::write(&slice, b"Slice").unwrap();
        let duplicate = registry
            .issue_slices(vec![slice.clone(), slice])
            .unwrap_err();
        assert_eq!(duplicate.code, "duplicate_slice");

        let original = root.path().join("original.bin.001.slice");
        let alias = root.path().join("alias.bin.001.slice");
        fs::write(&original, b"Slice alias").unwrap();
        fs::hard_link(&original, &alias).unwrap();
        let duplicate_identity = registry.issue_slices(vec![original, alias]).unwrap_err();
        assert_eq!(duplicate_identity.code, "duplicate_slice");

        let existing = root.path().join("rebuilt.bin");
        fs::write(&existing, b"occupied").unwrap();
        let collision = registry.issue_output_file(existing).unwrap_err();
        assert_eq!(collision.code, "output_collision");
    }

    #[test]
    fn package_folder_requires_exactly_one_manifest() {
        let root = tempdir().unwrap();
        let registry = SelectionRegistry::default();
        let folder = registry
            .issue_directory(root.path().to_path_buf(), SelectionKind::PackageFolder)
            .unwrap();

        let missing = manifest_from_selection(&registry, &folder.token).unwrap_err();
        assert_eq!(missing.code, "package_match_ambiguous");

        fs::write(root.path().join("a.cake.json"), b"{}").unwrap();
        assert_eq!(
            manifest_from_selection(&registry, &folder.token)
                .unwrap()
                .file_name()
                .unwrap(),
            "a.cake.json"
        );

        fs::write(root.path().join("b.cake.json"), b"{}").unwrap();
        let ambiguous = manifest_from_selection(&registry, &folder.token).unwrap_err();
        assert_eq!(ambiguous.code, "package_match_ambiguous");
    }

    #[test]
    fn rejects_same_name_and_same_size_file_replacements_without_rebinding_token() {
        let root = tempdir().unwrap();
        let registry = SelectionRegistry::default();
        let source = root.path().join("source.bin");
        let original = root.path().join("source-original.bin");
        fs::write(&source, b"original").unwrap();
        let selected = registry
            .issue_file(source.clone(), SelectionKind::SourceFile)
            .unwrap();

        fs::rename(&source, &original).unwrap();
        fs::write(&source, b"replaced").unwrap();
        let error = registry
            .resolve_one(&selected.token, &[SelectionKind::SourceFile])
            .unwrap_err();
        assert_eq!(error.code, "selection_identity_changed");
        fs::remove_file(&source).unwrap();
        fs::rename(&original, &source).unwrap();
        assert_eq!(
            registry
                .resolve_one(&selected.token, &[SelectionKind::SourceFile])
                .unwrap(),
            source
        );
    }

    #[test]
    fn rejects_replaced_manifest_and_package_directory() {
        let root = tempdir().unwrap();
        let registry = SelectionRegistry::default();
        let manifest = root.path().join("sample.cake.json");
        fs::write(&manifest, b"{}").unwrap();
        let manifest_selection = registry
            .issue_file(manifest.clone(), SelectionKind::ManifestFile)
            .unwrap();
        fs::remove_file(&manifest).unwrap();
        fs::write(&manifest, b"[]").unwrap();
        assert_eq!(
            manifest_from_selection(&registry, &manifest_selection.token)
                .unwrap_err()
                .code,
            "selection_identity_changed"
        );

        let package = root.path().join("package");
        let moved = root.path().join("package-original");
        fs::create_dir(&package).unwrap();
        fs::write(package.join("only.cake.json"), b"{}").unwrap();
        let package_selection = registry
            .issue_directory(package.clone(), SelectionKind::PackageFolder)
            .unwrap();
        fs::rename(&package, &moved).unwrap();
        fs::create_dir(&package).unwrap();
        fs::write(package.join("only.cake.json"), b"{}").unwrap();
        assert_eq!(
            manifest_from_selection(&registry, &package_selection.token)
                .unwrap_err()
                .code,
            "selection_identity_changed"
        );
    }

    #[test]
    fn rejects_one_or_all_replaced_slices_and_accepts_stable_set() {
        let root = tempdir().unwrap();
        let package = root.path().join("package");
        fs::create_dir(&package).unwrap();
        let first = package.join("sample.bin.001.slice");
        let second = package.join("sample.bin.002.slice");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();

        let stable_registry = SelectionRegistry::default();
        let stable = stable_registry
            .issue_slices(vec![first.clone(), second.clone()])
            .unwrap();
        assert_eq!(
            stable_registry
                .resolve_package_source(&stable.token)
                .unwrap(),
            package
        );

        let one_registry = SelectionRegistry::default();
        let one = one_registry
            .issue_slices(vec![first.clone(), second.clone()])
            .unwrap();
        fs::remove_file(&second).unwrap();
        fs::write(&second, b"second").unwrap();
        assert_eq!(
            one_registry
                .resolve_package_source(&one.token)
                .unwrap_err()
                .code,
            "selection_identity_changed"
        );

        let all_registry = SelectionRegistry::default();
        let all = all_registry
            .issue_slices(vec![first.clone(), second.clone()])
            .unwrap();
        let moved = root.path().join("package-original");
        fs::rename(&package, &moved).unwrap();
        fs::create_dir(&package).unwrap();
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        assert_eq!(
            all_registry
                .resolve_package_source(&all.token)
                .unwrap_err()
                .code,
            "selection_identity_changed"
        );
    }

    #[test]
    fn rejects_disappeared_selection_and_rebound_output_parent() {
        let root = tempdir().unwrap();
        let registry = SelectionRegistry::default();
        let source = root.path().join("source.bin");
        fs::write(&source, b"source").unwrap();
        let selected = registry
            .issue_file(source.clone(), SelectionKind::SourceFile)
            .unwrap();
        fs::remove_file(&source).unwrap();
        assert_eq!(
            registry
                .resolve_one(&selected.token, &[SelectionKind::SourceFile])
                .unwrap_err()
                .code,
            "selection_identity_changed"
        );

        let output_parent = root.path().join("output");
        let moved = root.path().join("output-original");
        fs::create_dir(&output_parent).unwrap();
        let output = output_parent.join("rebuilt.bin");
        let occupied_registry = SelectionRegistry::default();
        let occupied = occupied_registry.issue_output_file(output.clone()).unwrap();
        fs::write(&output, b"raced output").unwrap();
        assert_eq!(
            occupied_registry
                .resolve_one(&occupied.token, &[SelectionKind::OutputFile])
                .unwrap_err()
                .code,
            "selection_identity_changed"
        );
        fs::remove_file(&output).unwrap();
        let selected_output = registry.issue_output_file(output.clone()).unwrap();
        fs::rename(&output_parent, &moved).unwrap();
        fs::create_dir(&output_parent).unwrap();
        assert_eq!(
            registry
                .resolve_one(&selected_output.token, &[SelectionKind::OutputFile])
                .unwrap_err()
                .code,
            "selection_identity_changed"
        );
    }
}
