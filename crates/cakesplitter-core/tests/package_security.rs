use std::{fs, path::Path};

use cakesplitter_core::{
    CancellationToken, CoreError, MAX_PACKAGE_DIAGNOSTIC_ENTRIES, SplitOptions,
    capture_package_binding, inspect_package, inspect_package_bound, split_file,
};
use tempfile::tempdir;

#[test]
fn durable_binding_rejects_manifest_slice_set_and_directory_replacements() {
    let root = tempdir().unwrap();
    let package_a = root.path().join("package-a");
    let package_b = root.path().join("package-b");
    fs::create_dir(&package_a).unwrap();
    fs::create_dir(&package_b).unwrap();
    let source_a = root.path().join("source-a").join("sample.bin");
    let source_b = root.path().join("source-b").join("sample.bin");
    fs::create_dir_all(source_a.parent().unwrap()).unwrap();
    fs::create_dir_all(source_b.parent().unwrap()).unwrap();
    fs::write(&source_a, vec![0x41; 96]).unwrap();
    fs::write(&source_b, vec![0x42; 96]).unwrap();
    let manifest_a = split_file(
        &source_a,
        &SplitOptions {
            slice_size: 32,
            output_dir: package_a.clone(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let manifest_b = split_file(
        &source_b,
        &SplitOptions {
            slice_size: 32,
            output_dir: package_b.clone(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let binding = capture_package_binding(&manifest_a, &CancellationToken::new()).unwrap();
    assert!(
        inspect_package_bound(&manifest_a, true, &binding, &CancellationToken::new())
            .unwrap()
            .verified
    );

    let original_manifest = root.path().join("original-manifest.cake.json");
    fs::rename(&manifest_a, &original_manifest).unwrap();
    fs::copy(&manifest_b, &manifest_a).unwrap();
    assert_package_changed(&manifest_a, &binding);
    fs::remove_file(&manifest_a).unwrap();
    fs::rename(&original_manifest, &manifest_a).unwrap();

    let first_slice = package_a.join(&binding.manifest.slices[0].filename);
    let original_slice = root.path().join("original-first.slice");
    fs::rename(&first_slice, &original_slice).unwrap();
    fs::write(&first_slice, vec![0x41; 32]).unwrap();
    assert_package_changed(&manifest_a, &binding);
    fs::remove_file(&first_slice).unwrap();
    fs::rename(&original_slice, &first_slice).unwrap();

    let stash = root.path().join("package-a-original-files");
    fs::create_dir(&stash).unwrap();
    move_entries(&package_a, &stash);
    copy_entries(&package_b, &package_a);
    assert_package_changed(&manifest_a, &binding);
    remove_entries(&package_a);
    move_entries(&stash, &package_a);
    assert!(
        inspect_package_bound(&manifest_a, true, &binding, &CancellationToken::new())
            .unwrap()
            .verified
    );

    let moved_directory = root.path().join("package-a-original-directory");
    fs::rename(&package_a, &moved_directory).unwrap();
    fs::create_dir(&package_a).unwrap();
    copy_entries(&package_b, &package_a);
    assert_package_changed(&manifest_a, &binding);
}

#[test]
fn durable_binding_rejects_added_removed_and_ambiguous_membership_without_rebinding() {
    let root = tempdir().unwrap();
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    let source = root.path().join("membership.bin");
    fs::write(&source, vec![0x61; 64]).unwrap();
    let manifest = split_file(
        &source,
        &SplitOptions {
            slice_size: 16,
            output_dir: package.clone(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let binding = capture_package_binding(&manifest, &CancellationToken::new()).unwrap();

    let extra = package.join("unexpected.001.slice");
    fs::write(&extra, b"unexpected").unwrap();
    assert_package_changed(&manifest, &binding);
    fs::remove_file(extra).unwrap();

    let removed = package.join(&binding.manifest.slices[1].filename);
    let stash = root.path().join("removed.slice");
    fs::rename(&removed, &stash).unwrap();
    assert_package_changed(&manifest, &binding);
    fs::rename(&stash, &removed).unwrap();

    assert!(
        inspect_package_bound(&manifest, true, &binding, &CancellationToken::new())
            .unwrap()
            .verified
    );
}

#[test]
fn package_enumeration_accepts_exact_diagnostic_limit_rejects_excess_and_never_recurses() {
    let root = tempdir().unwrap();
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    let source = root.path().join("enumeration.bin");
    fs::write(&source, b"bounded package").unwrap();
    let manifest = split_file(
        &source,
        &SplitOptions {
            slice_size: 1024,
            output_dir: package.clone(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let nested = package.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("not-enumerated.slice"), b"nested").unwrap();
    for index in 0..MAX_PACKAGE_DIAGNOSTIC_ENTRIES {
        fs::write(package.join(format!("unexpected-{index:04}.slice")), b"x").unwrap();
    }
    let exact = inspect_package(&manifest, false, &CancellationToken::new()).unwrap();
    assert_eq!(exact.unexpected.len(), MAX_PACKAGE_DIAGNOSTIC_ENTRIES);
    assert!(
        !exact
            .unexpected
            .iter()
            .any(|name| name == "not-enumerated.slice")
    );

    fs::write(package.join("unexpected-over-limit.slice"), b"x").unwrap();
    assert!(matches!(
        inspect_package(&manifest, false, &CancellationToken::new()),
        Err(CoreError::PackageEnumerationLimit {
            resource: "unexpected Slice diagnostic count",
            ..
        })
    ));

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        inspect_package(&manifest, false, &cancelled),
        Err(CoreError::Cancelled)
    ));
}

fn assert_package_changed(manifest: &Path, binding: &cakesplitter_core::PackageBinding) {
    assert!(matches!(
        inspect_package_bound(manifest, true, binding, &CancellationToken::new()),
        Err(CoreError::PackageIdentityChanged(_))
            | Err(CoreError::InvalidManifest(_))
            | Err(CoreError::InvalidJson(_))
    ));
}

fn move_entries(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        fs::rename(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn copy_entries(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn remove_entries(directory: &Path) {
    for entry in fs::read_dir(directory).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
}
