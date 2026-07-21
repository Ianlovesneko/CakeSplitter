use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;
use tempfile::tempdir;

#[test]
fn help_and_explicit_version_are_stable() {
    let help = cli().arg("help").output().unwrap();
    assert!(help.status.success());
    let help = text(&help.stdout);
    for command in [
        "split", "merge", "inspect", "verify", "plan", "version", "help",
    ] {
        assert!(help.contains(command), "missing {command} in help");
    }

    let version = cli()
        .args(["version", "--format", "json"])
        .output()
        .unwrap();
    assert!(version.status.success());
    assert!(version.stderr.is_empty());
    let version = json(&version.stdout);
    assert_eq!(version["schemaVersion"], 1);
    assert_eq!(version["result"]["applicationVersion"], "0.6.0-dev");
    assert_eq!(version["result"]["cakePackageFormat"], "1.0");
}

#[test]
fn plan_and_split_dry_run_leave_the_filesystem_unchanged() {
    let root = tempdir().unwrap();
    let source = root.path().join("dry run source.bin");
    let output = root.path().join("future package");
    fs::write(&source, b"dry-run fixture").unwrap();
    let before = inventory(root.path());

    for arguments in [
        vec![
            "plan",
            "split",
            source.to_str().unwrap(),
            "--slice-size",
            "4B",
            "--output-dir",
            output.to_str().unwrap(),
            "--format",
            "json",
        ],
        vec![
            "split",
            source.to_str().unwrap(),
            "--slice-size",
            "4B",
            "--output-dir",
            output.to_str().unwrap(),
            "--dry-run",
            "--format",
            "json",
        ],
    ] {
        let result = cli().args(arguments).output().unwrap();
        assert!(result.status.success(), "{}", text(&result.stderr));
        assert!(result.stderr.is_empty());
        let result = json(&result.stdout);
        assert_eq!(result["status"], "completed");
        assert_eq!(result["result"]["dryRun"], true);
        assert_eq!(inventory(root.path()), before);
        assert!(!output.exists());
    }
}

#[test]
fn jsonl_split_and_json_inspect_verify_merge_round_trip() {
    let root = tempdir().unwrap();
    let source = root.path().join("archive.tar.bin");
    let package = root.path().join("package with spaces");
    let rebuilt = root.path().join("rebuilt archive.bin");
    fs::write(
        &source,
        (0_u8..=250).cycle().take(32_000).collect::<Vec<_>>(),
    )
    .unwrap();
    fs::create_dir(&package).unwrap();

    let split = cli()
        .arg("split")
        .arg(&source)
        .args(["--slice-count", "3", "--output-dir"])
        .arg(&package)
        .args(["--format", "jsonl"])
        .output()
        .unwrap();
    assert!(split.status.success(), "{}", text(&split.stderr));
    assert!(split.stderr.is_empty());
    let events = text(&split.stdout)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.first().unwrap()["event"], "started");
    assert!(events.iter().any(|event| event["event"] == "preflight"));
    assert!(events.iter().any(|event| event["event"] == "progress"));
    assert_eq!(events.last().unwrap()["event"], "completed");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event["event"].as_str(),
                Some("completed" | "failed" | "cancelled")
            ))
            .count(),
        1
    );
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence"], (index + 1) as u64);
    }

    let manifest = only_suffix(&package, ".cake.json");
    let inspect = machine(&["inspect", manifest.to_str().unwrap()]);
    assert_eq!(inspect["status"], "completed");
    assert_eq!(inspect["result"]["ready"], true);
    assert_eq!(inspect["result"]["verified"], false);

    let verify = machine(&["verify", manifest.to_str().unwrap()]);
    assert_eq!(verify["status"], "completed");
    assert_eq!(verify["result"]["verified"], true);

    let mut slice_paths = fs::read_dir(&package)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(OsStr::to_str) == Some("slice"))
        .collect::<Vec<_>>();
    slice_paths.sort();
    let mut merge_command = cli();
    merge_command
        .arg("merge")
        .arg(&package)
        .args(["--output"])
        .arg(&rebuilt);
    for slice in &slice_paths {
        merge_command.arg("--slice").arg(slice);
    }
    let merge = merge_command.args(["--format", "json"]).output().unwrap();
    assert!(merge.status.success(), "{}", text(&merge.stderr));
    assert!(merge.stderr.is_empty());
    assert_eq!(json(&merge.stdout)["result"]["verified"], true);
    assert_eq!(fs::read(&rebuilt).unwrap(), fs::read(&source).unwrap());
}

#[test]
fn no_overwrite_and_argument_failures_are_parseable() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let package = root.path().join("package");
    fs::write(&source, b"collision fixture").unwrap();
    fs::create_dir(&package).unwrap();

    let first = cli()
        .arg("split")
        .arg(&source)
        .args(["--slice-size", "4B", "--output-dir"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(first.status.success(), "{}", text(&first.stderr));

    let collision = cli()
        .arg("split")
        .arg(&source)
        .args(["--slice-size", "4B", "--output-dir"])
        .arg(&package)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(collision.status.code(), Some(4));
    assert!(collision.stderr.is_empty());
    let collision = json(&collision.stdout);
    assert_eq!(collision["error"]["category"], "conflict");
    assert_eq!(collision["error"]["retryable"], true);

    for invalid in ["0", "1KB", "-1", "1.5MiB"] {
        let output = cli()
            .arg("split")
            .arg(&source)
            .args(["--slice-size", invalid, "--format", "json"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{invalid}");
        assert!(output.stderr.is_empty());
        assert_eq!(json(&output.stdout)["error"]["category"], "usage");
    }
}

#[test]
fn explicit_receipt_is_redacted_and_receipt_failure_does_not_reverse_success() {
    let root = tempdir().unwrap();
    let source = root.path().join("receipt source.bin");
    let package = root.path().join("package");
    let receipt = root.path().join("split-receipt.json");
    fs::write(&source, b"receipt fixture").unwrap();
    fs::create_dir(&package).unwrap();

    let split = cli()
        .arg("split")
        .arg(&source)
        .args(["--slice-size", "4B", "--output-dir"])
        .arg(&package)
        .args(["--receipt"])
        .arg(&receipt)
        .args(["--receipt-format", "json", "--format", "json"])
        .output()
        .unwrap();
    assert!(split.status.success(), "{}", text(&split.stderr));
    let result = json(&split.stdout);
    assert_eq!(result["result"]["receipt"]["status"], "completed");
    let receipt_text = fs::read_to_string(&receipt).unwrap();
    assert!(!receipt_text.contains(&root.path().display().to_string()));
    assert_eq!(
        json(receipt_text.as_bytes())["privacy"]["pathsMasked"],
        true
    );

    let manifest = only_suffix(&package, ".cake.json");
    let verify = cli()
        .arg("verify")
        .arg(&manifest)
        .args(["--receipt"])
        .arg(&receipt)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "receipt collision must not reverse verification success"
    );
    let verify = json(&verify.stdout);
    assert_eq!(verify["status"], "completed");
    assert_eq!(verify["result"]["receipt"]["status"], "failed");
    assert_eq!(verify["warnings"].as_array().unwrap().len(), 1);
}

#[test]
fn merge_plan_reports_unverified_hashes_when_a_slice_is_missing() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let package = root.path().join("package");
    fs::write(&source, b"plan integrity fixture").unwrap();
    fs::create_dir(&package).unwrap();
    let split = cli()
        .arg("split")
        .arg(&source)
        .args(["--slice-size", "4B", "--output-dir"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(split.status.success(), "{}", text(&split.stderr));
    let slice = fs::read_dir(&package)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(OsStr::to_str) == Some("slice"))
        .unwrap();
    fs::remove_file(slice).unwrap();
    let output = root.path().join("rebuilt.bin");
    let plan = cli()
        .args(["plan", "merge"])
        .arg(&package)
        .args(["--output"])
        .arg(&output)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(plan.status.success(), "{}", text(&plan.stderr));
    assert_eq!(
        json(&plan.stdout)["result"]["plan"]["hashesVerified"],
        false
    );
}

#[test]
fn explicit_dry_run_receipt_is_the_only_planned_output() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let output = root.path().join("future-package");
    let receipt = root.path().join("dry-run.md");
    fs::write(&source, b"dry run receipt fixture").unwrap();
    let before = inventory(root.path());
    let result = cli()
        .args(["split"])
        .arg(&source)
        .args(["--slice-size", "4B", "--output-dir"])
        .arg(&output)
        .args(["--dry-run", "--receipt"])
        .arg(&receipt)
        .args(["--receipt-format", "markdown", "--format", "json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(
        json(&result.stdout)["result"]["receipt"]["status"],
        "dry-run"
    );
    assert!(receipt.exists());
    assert!(!output.exists());
    let mut after = inventory(root.path());
    after.retain(|name| name != "dry-run.md");
    assert_eq!(after, before);
    assert!(
        String::from_utf8(fs::read(&receipt).unwrap())
            .unwrap()
            .contains("Status: `dry-run`")
    );
}

#[test]
fn duplicate_manifest_is_rejected_as_structured_package_input() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let package = root.path().join("package");
    fs::write(&source, b"duplicate manifest fixture").unwrap();
    fs::create_dir(&package).unwrap();
    let split = cli()
        .arg("split")
        .arg(&source)
        .args(["--slice-size", "4B", "--output-dir"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(split.status.success(), "{}", text(&split.stderr));
    let manifest = only_suffix(&package, ".cake.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    let slices = value["slices"].as_array_mut().unwrap();
    if slices.len() < 2 {
        return;
    }
    slices[1]["index"] = slices[0]["index"].clone();
    let duplicate = root.path().join("duplicate.cake.json");
    fs::write(&duplicate, serde_json::to_vec(&value).unwrap()).unwrap();
    let result = cli()
        .args(["verify"])
        .arg(&duplicate)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(json(&result.stdout)["error"]["category"], "package");
}

#[cfg(windows)]
#[test]
fn reparse_source_and_destination_paths_fail_closed() {
    use std::os::windows::fs::{symlink_dir, symlink_file};

    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let source_link = root.path().join("source-link.bin");
    let real_output = root.path().join("real-output");
    let output_link = root.path().join("output-link");
    fs::write(&source, b"reparse fixture").unwrap();
    fs::create_dir(&real_output).unwrap();
    if symlink_file(&source, &source_link).is_err()
        || symlink_dir(&real_output, &output_link).is_err()
    {
        return;
    }
    let source_result = cli()
        .args(["split"])
        .arg(&source_link)
        .args(["--slice-size", "4B", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(source_result.status.code(), Some(5));
    assert_eq!(
        json(&source_result.stdout)["error"]["code"],
        "unsafe_filesystem_path"
    );
    let destination_result = cli()
        .args(["split"])
        .arg(&source)
        .args(["--slice-size", "4B", "--output-dir"])
        .arg(&output_link)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(destination_result.status.code(), Some(6));
    assert_eq!(
        json(&destination_result.stdout)["error"]["category"],
        "destination"
    );
    assert!(fs::read_dir(&real_output).unwrap().next().is_none());
}

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cakesplitter"))
}

fn machine(arguments: &[&str]) -> Value {
    let output = cli()
        .args(arguments)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(output.stderr.is_empty());
    json(&output.stdout)
}

fn json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|error| panic!("invalid JSON: {error}: {}", String::from_utf8_lossy(bytes)))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn inventory(path: &Path) -> Vec<String> {
    let mut entries = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn only_suffix(path: &Path, suffix: &str) -> PathBuf {
    let matches = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.ends_with(suffix))
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1);
    matches.into_iter().next().unwrap()
}
