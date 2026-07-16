use std::{fs, path::Path};

use cakesplitter_core::{
    CancellationToken, CoreError, SplitOptions, inspect_package, merge_package,
    merge_package_with_progress, split_file, split_file_with_progress, verify_package,
};
use tempfile::tempdir;

fn round_trip(filename: &str, bytes: &[u8], slice_size: u64) {
    let input_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let input = input_dir.path().join(filename);
    fs::write(&input, bytes).unwrap();
    let token = CancellationToken::new();
    let manifest = split_file(
        &input,
        &SplitOptions {
            slice_size,
            output_dir: output_dir.path().to_path_buf(),
            cancellation: token.clone(),
        },
    )
    .unwrap();
    let inspection = verify_package(&manifest, &token).unwrap();
    assert!(inspection.verified);
    let rebuilt = output_dir.path().join(format!("rebuilt-{filename}"));
    merge_package(&manifest, &rebuilt, &token).unwrap();
    assert_eq!(fs::read(rebuilt).unwrap(), bytes);
}

#[test]
fn round_trips_required_size_boundaries() {
    for (name, bytes, slice_size) in [
        ("empty.bin", vec![], 4),
        ("one.bin", vec![7], 4),
        ("smaller.bin", vec![1, 2, 3], 4),
        ("exact.bin", vec![1, 2, 3, 4], 4),
        ("larger.bin", vec![1, 2, 3, 4, 5], 4),
        ("many.bin", (0_u8..17).collect(), 4),
    ] {
        round_trip(name, &bytes, slice_size);
    }
}

#[test]
fn round_trips_unicode_and_multiple_extensions() {
    round_trip("生日蛋糕.archive.tar", b"layers-and-frosting", 5);
}

#[test]
fn detects_missing_modified_and_unexpected_slices() {
    let input_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let input = input_dir.path().join("cake.bin");
    fs::write(&input, b"0123456789").unwrap();
    let token = CancellationToken::new();
    let manifest_path = split_file(
        &input,
        &SplitOptions {
            slice_size: 4,
            output_dir: output_dir.path().to_path_buf(),
            cancellation: token.clone(),
        },
    )
    .unwrap();
    let manifest = cakesplitter_core::load_manifest(&manifest_path).unwrap();

    fs::remove_file(output_dir.path().join(&manifest.slices[0].filename)).unwrap();
    fs::write(
        output_dir.path().join(&manifest.slices[1].filename),
        b"xxxx",
    )
    .unwrap();
    fs::write(output_dir.path().join("stray.001.slice"), b"noise").unwrap();

    let inspection = verify_package(&manifest_path, &token).unwrap();
    assert_eq!(
        inspection.missing,
        vec![manifest.slices[0].filename.clone()]
    );
    assert_eq!(
        inspection.corrupted,
        vec![manifest.slices[1].filename.clone()]
    );
    assert_eq!(inspection.unexpected, vec!["stray.001.slice"]);
    assert!(!inspection.verified);
}

#[test]
fn cancelled_split_leaves_no_complete_package() {
    let input_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let input = input_dir.path().join("cancel.bin");
    fs::write(&input, vec![0_u8; 4096]).unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let result = split_file(
        &input,
        &SplitOptions {
            slice_size: 1024,
            output_dir: output_dir.path().to_path_buf(),
            cancellation: token,
        },
    );
    assert!(matches!(result, Err(CoreError::Cancelled)));
    assert!(directory_files(output_dir.path()).is_empty());
}

#[test]
fn does_not_overwrite_existing_outputs() {
    let input_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let input = input_dir.path().join("collision.bin");
    fs::write(&input, b"abcdef").unwrap();
    let existing = output_dir.path().join("collision.bin.001.slice");
    fs::write(&existing, b"keep-me").unwrap();
    let result = split_file(
        &input,
        &SplitOptions {
            slice_size: 3,
            output_dir: output_dir.path().to_path_buf(),
            cancellation: CancellationToken::new(),
        },
    );
    assert!(matches!(result, Err(CoreError::Collision(path)) if path == existing));
    assert_eq!(fs::read(existing).unwrap(), b"keep-me");
}

#[test]
fn split_finalization_does_not_replace_raced_in_slice() {
    let input_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let input = input_dir.path().join("race.bin");
    fs::write(&input, b"abcdef").unwrap();
    let raced_in = output_dir.path().join("race.bin.001.slice");
    let result = split_file_with_progress(
        &input,
        &SplitOptions {
            slice_size: 3,
            output_dir: output_dir.path().to_path_buf(),
            cancellation: CancellationToken::new(),
        },
        |progress| {
            if progress.bytes_processed == 3 && !raced_in.exists() {
                fs::write(&raced_in, b"sentinel").unwrap();
            }
        },
    );
    assert!(matches!(result, Err(CoreError::Collision(path)) if path == raced_in));
    assert_eq!(fs::read(raced_in).unwrap(), b"sentinel");
    assert!(!output_dir.path().join("race.bin.cake.json").exists());
}

#[test]
fn split_finalization_does_not_replace_raced_in_manifest() {
    let input_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let input = input_dir.path().join("manifest-race.bin");
    fs::write(&input, b"abcdef").unwrap();
    let raced_in = output_dir.path().join("manifest-race.bin.cake.json");
    let result = split_file_with_progress(
        &input,
        &SplitOptions {
            slice_size: 3,
            output_dir: output_dir.path().to_path_buf(),
            cancellation: CancellationToken::new(),
        },
        |progress| {
            if progress.bytes_processed == progress.total_bytes && !raced_in.exists() {
                fs::write(&raced_in, b"sentinel manifest").unwrap();
            }
        },
    );
    assert!(matches!(result, Err(CoreError::Collision(path)) if path == raced_in));
    assert_eq!(fs::read(raced_in).unwrap(), b"sentinel manifest");
}

#[test]
fn merge_finalization_does_not_replace_raced_in_output() {
    let input_dir = tempdir().unwrap();
    let package_dir = tempdir().unwrap();
    let input = input_dir.path().join("merge-race.bin");
    fs::write(&input, b"abcdef").unwrap();
    let manifest = split_file(
        &input,
        &SplitOptions {
            slice_size: 3,
            output_dir: package_dir.path().to_path_buf(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let output = package_dir.path().join("rebuilt.bin");
    let result =
        merge_package_with_progress(&manifest, &output, &CancellationToken::new(), |progress| {
            if progress.bytes_processed == progress.total_bytes && !output.exists() {
                fs::write(&output, b"sentinel output").unwrap();
            }
        });
    assert!(matches!(result, Err(CoreError::Collision(path)) if path == output));
    assert_eq!(fs::read(output).unwrap(), b"sentinel output");
}

#[test]
fn split_rejects_a_rebound_partial_and_preserves_the_replacement() {
    let input_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let input = input_dir.path().join("substitute.bin");
    fs::write(&input, b"abcdef").unwrap();
    let partial = output_dir.path().join("substitute.bin.001.slice.partial");
    let result = split_file_with_progress(
        &input,
        &SplitOptions {
            slice_size: 3,
            output_dir: output_dir.path().to_path_buf(),
            cancellation: CancellationToken::new(),
        },
        |progress| {
            if progress.bytes_processed == 3 && partial.exists() {
                fs::remove_file(&partial).unwrap();
                fs::write(&partial, b"replacement").unwrap();
            }
        },
    );
    assert!(matches!(
        result,
        Err(CoreError::StagedIdentityChanged(path)) if path == partial
    ));
    assert_eq!(fs::read(partial).unwrap(), b"replacement");
    assert!(!output_dir.path().join("substitute.bin.001.slice").exists());
}

#[test]
fn merge_rejects_a_rebound_partial_and_preserves_the_replacement() {
    let input_dir = tempdir().unwrap();
    let package_dir = tempdir().unwrap();
    let input = input_dir.path().join("merge-substitute.bin");
    fs::write(&input, b"abcdef").unwrap();
    let manifest = split_file(
        &input,
        &SplitOptions {
            slice_size: 3,
            output_dir: package_dir.path().to_path_buf(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let output = package_dir.path().join("rebuilt-substitute.bin");
    let partial = package_dir.path().join("rebuilt-substitute.bin.partial");
    let result =
        merge_package_with_progress(&manifest, &output, &CancellationToken::new(), |progress| {
            if progress.bytes_processed == progress.total_bytes && partial.exists() {
                fs::remove_file(&partial).unwrap();
                fs::write(&partial, b"replacement").unwrap();
            }
        });
    assert!(matches!(
        result,
        Err(CoreError::StagedIdentityChanged(path)) if path == partial
    ));
    assert_eq!(fs::read(partial).unwrap(), b"replacement");
    assert!(!output.exists());
}

#[test]
fn inspect_without_hashing_reports_completeness_but_not_verified() {
    let input_dir = tempdir().unwrap();
    let output_dir = tempdir().unwrap();
    let input = input_dir.path().join("inspect.bin");
    fs::write(&input, b"abcdef").unwrap();
    let manifest = split_file(
        &input,
        &SplitOptions {
            slice_size: 3,
            output_dir: output_dir.path().to_path_buf(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let inspection = inspect_package(&manifest, false, &CancellationToken::new()).unwrap();
    assert_eq!(inspection.found_slice_count, 2);
    assert!(inspection.missing.is_empty());
    assert!(!inspection.verified);
}

fn directory_files(path: &Path) -> Vec<String> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}
