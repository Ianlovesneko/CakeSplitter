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
    assert_eq!(version["result"]["applicationVersion"], "0.8.1");
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

#[test]
fn batch_run_persists_bounded_state_and_completion() {
    let root = tempdir().unwrap();
    let source = root.path().join("batch-source.bin");
    let package = root.path().join("batch-package");
    let state = root.path().join("batch-run.json");
    let receipt = root.path().join("batch-receipt.json");
    fs::write(&source, b"batch workflow fixture").unwrap();
    fs::create_dir(&package).unwrap();
    let spec = root.path().join("batch-job.json");
    fs::write(
        &spec,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 1,
            "name": "batch-workflow",
            "failurePolicy": "stop",
            "operations": [
                { "id": "split", "command": "split", "file": source, "sliceSize": "4B", "outputDir": package }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let validate = cli()
        .args(["batch", "validate"])
        .arg(&spec)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(validate.status.success(), "{}", text(&validate.stderr));
    assert_eq!(json(&validate.stdout)["result"]["operationCount"], 1);
    assert_eq!(json(&validate.stdout)["command"], "batch");
    assert!(json(&validate.stdout)["runId"].as_str().is_some());
    assert_eq!(json(&validate.stdout)["operationCounts"]["not-started"], 1);

    let plan = cli()
        .args(["batch", "plan"])
        .arg(&spec)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(plan.status.success(), "{}", text(&plan.stderr));
    assert_eq!(json(&plan.stdout)["result"]["ready"], true);
    assert_eq!(json(&plan.stdout)["command"], "batch");
    assert_eq!(json(&plan.stdout)["operations"][0]["attemptCount"], 0);

    let run = cli()
        .args(["batch", "run"])
        .arg(&spec)
        .args(["--state"])
        .arg(&state)
        .args(["--receipt"])
        .arg(&receipt)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", text(&run.stderr));
    assert!(run.stderr.is_empty());
    assert_eq!(json(&run.stdout)["status"], "completed");
    assert_eq!(json(&run.stdout)["command"], "batch");
    assert!(json(&run.stdout)["jobSpecDigest"].as_str().is_some());
    assert_eq!(json(&run.stdout)["operationCounts"]["completed"], 1);
    assert_eq!(
        json(&run.stdout)["result"]["receipt"]["status"],
        "completed"
    );
    let receipt_text = fs::read_to_string(&receipt).unwrap();
    assert!(!receipt_text.contains(&root.path().display().to_string()));
    assert_eq!(
        json(receipt_text.as_bytes())["privacy"]["pathsMasked"],
        true
    );
    let stored: Value = serde_json::from_slice(&fs::read(&state).unwrap()).unwrap();
    assert_eq!(stored["state"]["terminalState"], "completed");
    assert_eq!(stored["state"]["operations"][0]["status"], "completed");
    assert_eq!(stored["state"]["operations"].as_array().unwrap().len(), 1);

    let status = cli()
        .args(["batch", "status"])
        .arg(&state)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(status.status.success(), "{}", text(&status.stderr));
    assert_eq!(json(&status.stdout)["result"]["terminalState"], "completed");

    let resume = cli()
        .args(["batch", "resume"])
        .arg(&state)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(resume.status.success(), "{}", text(&resume.stderr));
    assert_eq!(json(&resume.stdout)["status"], "completed");
}

#[test]
fn batch_plan_is_read_only_and_reports_deterministic_order() {
    let root = tempdir().unwrap();
    let source = root.path().join("plan-source.bin");
    let output = root.path().join("plan-output");
    let spec = root.path().join("plan-job.json");
    fs::write(&source, b"plan fixture").unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(&spec, serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1,
        "name": "plan-only",
        "failurePolicy": "continue-independent",
        "operations": [{ "id": "split", "command": "split", "file": source, "sliceSize": "4B", "outputDir": output }]
    })).unwrap()).unwrap();
    let before = inventory(root.path());
    let result = cli()
        .args(["batch", "plan"])
        .arg(&spec)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", text(&result.stderr));
    assert_eq!(json(&result.stdout)["result"]["ready"], true);
    assert_eq!(
        json(&result.stdout)["result"]["executionOrder"],
        serde_json::json!(["split"])
    );
    assert_eq!(inventory(root.path()), before);
    assert!(!output.join("plan-source.bin.cake.json").exists());

    let jsonl = cli()
        .args(["batch", "plan"])
        .arg(&spec)
        .args(["--format", "jsonl"])
        .output()
        .unwrap();
    assert!(jsonl.status.success(), "{}", text(&jsonl.stderr));
    let events = text(&jsonl.stdout)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.first().unwrap()["event"], "started");
    assert_eq!(events.last().unwrap()["event"], "completed");
    assert!(events.iter().all(|event| {
        event["runId"].as_str().is_some() && event["payload"]["runId"] == event["runId"]
    }));
}

#[test]
fn batch_rejects_cycles_duplicates_forbidden_commands_and_oversized_graphs() {
    let root = tempdir().unwrap();
    for (name, value, code) in [
        (
            "cycle.json",
            serde_json::json!({
                "schemaVersion": 1,
                "name": "cycle",
                "failurePolicy": "stop",
                "operations": [
                    { "id": "a", "command": "inspect", "package": "a.cake.json", "dependsOn": ["b"] },
                    { "id": "b", "command": "inspect", "package": "b.cake.json", "dependsOn": ["a"] }
                ]
            }),
            "batch_dependency_cycle",
        ),
        (
            "duplicate.json",
            serde_json::json!({
                "schemaVersion": 1,
                "name": "duplicate",
                "failurePolicy": "stop",
                "operations": [
                    { "id": "same", "command": "inspect", "package": "a.cake.json" },
                    { "id": "same", "command": "inspect", "package": "b.cake.json" }
                ]
            }),
            "batch_duplicate_operation_id",
        ),
        (
            "shell.json",
            serde_json::json!({
                "schemaVersion": 1,
                "name": "shell",
                "failurePolicy": "stop",
                "operations": [{ "id": "shell", "command": "shell", "commandLine": "echo nope" }]
            }),
            "batch_invalid_schema",
        ),
    ] {
        let spec = root.path().join(name);
        fs::write(&spec, serde_json::to_vec(&value).unwrap()).unwrap();
        let result = cli()
            .args(["batch", "validate"])
            .arg(&spec)
            .args(["--format", "json"])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2), "{}", text(&result.stderr));
        assert_eq!(json(&result.stdout)["error"]["code"], code);
    }

    let oversized_operations = (0..1_001)
        .map(|index| {
            serde_json::json!({
                "id": format!("op-{index}"),
                "command": "inspect",
                "package": "package.cake.json"
            })
        })
        .collect::<Vec<_>>();
    let oversized = root.path().join("oversized.json");
    fs::write(
        &oversized,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "name": "oversized",
            "failurePolicy": "stop",
            "operations": oversized_operations
        }))
        .unwrap(),
    )
    .unwrap();
    let result = cli()
        .args(["batch", "validate"])
        .arg(&oversized)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(10));
    assert_eq!(
        json(&result.stdout)["error"]["code"],
        "batch_operation_limit"
    );

    let oversized_metadata = root.path().join("oversized-metadata.json");
    fs::write(
        &oversized_metadata,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "name": "metadata",
            "failurePolicy": "stop",
            "metadata": "x".repeat(64 * 1024),
            "operations": [{ "id": "inspect", "command": "inspect", "package": "package.cake.json" }]
        }))
        .unwrap(),
    )
    .unwrap();
    let result = cli()
        .args(["batch", "validate"])
        .arg(&oversized_metadata)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(10));
    assert_eq!(
        json(&result.stdout)["error"]["code"],
        "batch_metadata_limit"
    );
}

#[test]
fn batch_resume_rejects_spec_digest_substitution_and_corrupt_state() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let package = root.path().join("package");
    let state = root.path().join("state.json");
    fs::write(&source, b"digest fixture").unwrap();
    fs::create_dir(&package).unwrap();
    let spec = root.path().join("job.json");
    fs::write(&spec, serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1,
        "name": "digest",
        "failurePolicy": "stop",
        "operations": [{ "id": "split", "command": "split", "file": source, "sliceSize": "4B", "outputDir": package }]
    })).unwrap()).unwrap();
    let run = cli()
        .args(["batch", "run"])
        .arg(&spec)
        .args(["--state"])
        .arg(&state)
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", text(&run.stderr));
    let mut changed: Value = serde_json::from_slice(&fs::read(&spec).unwrap()).unwrap();
    changed["name"] = Value::String("changed".to_owned());
    fs::write(&spec, serde_json::to_vec(&changed).unwrap()).unwrap();
    let mismatch = cli()
        .args(["batch", "resume"])
        .arg(&state)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(mismatch.status.code(), Some(9));
    assert_eq!(
        json(&mismatch.stdout)["error"]["code"],
        "batch_spec_digest_mismatch"
    );

    fs::write(&state, b"not-json").unwrap();
    let corrupt = cli()
        .args(["batch", "status"])
        .arg(&state)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(corrupt.status.code(), Some(9));
    assert_eq!(
        json(&corrupt.stdout)["error"]["code"],
        "batch_state_corrupt"
    );
}

#[test]
fn batch_continue_independent_runs_ready_operations_after_a_runtime_failure() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let shared_output = root.path().join("shared-output");
    let independent_output = root.path().join("independent-output");
    let state = root.path().join("continue.json");
    let spec = root.path().join("continue-job.json");
    fs::write(&source, b"continue-independent fixture").unwrap();
    fs::create_dir(&shared_output).unwrap();
    fs::create_dir(&independent_output).unwrap();
    fs::write(
        &spec,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "name": "continue-independent",
            "failurePolicy": "continue-independent",
            "operations": [
                { "id": "first", "command": "split", "file": source, "sliceSize": "4B", "outputDir": shared_output },
                { "id": "collision", "command": "split", "file": source, "sliceSize": "4B", "outputDir": shared_output },
                { "id": "independent", "command": "split", "file": source, "sliceSize": "4B", "outputDir": independent_output }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let result = cli()
        .args(["batch", "run"])
        .arg(&spec)
        .args(["--state"])
        .arg(&state)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(11), "{}", text(&result.stderr));
    let summary = json(&result.stdout);
    assert_eq!(summary["status"], "completed-with-failures");
    let operations = summary["result"]["operations"].as_array().unwrap();
    assert_eq!(operations[0]["status"], "completed");
    assert_eq!(operations[1]["status"], "failed");
    assert_eq!(operations[2]["status"], "completed");
    assert!(independent_output.join("source.bin.cake.json").exists());
}

#[test]
fn batch_stop_policy_blocks_operations_after_the_first_runtime_failure() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let output = root.path().join("output");
    let untouched = root.path().join("untouched");
    let state = root.path().join("stop.json");
    let spec = root.path().join("stop-job.json");
    fs::write(&source, b"stop-policy fixture").unwrap();
    fs::create_dir(&output).unwrap();
    fs::create_dir(&untouched).unwrap();
    fs::write(
        &spec,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "name": "stop-policy",
            "failurePolicy": "stop",
            "operations": [
                { "id": "first", "command": "split", "file": source, "sliceSize": "4B", "outputDir": output },
                { "id": "collision", "command": "split", "file": source, "sliceSize": "4B", "outputDir": output },
                { "id": "blocked", "command": "split", "file": source, "sliceSize": "4B", "outputDir": untouched }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let result = cli()
        .args(["batch", "run"])
        .arg(&spec)
        .args(["--state"])
        .arg(&state)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(11), "{}", text(&result.stderr));
    let summary = json(&result.stdout);
    let operations = summary["result"]["operations"].as_array().unwrap();
    assert_eq!(operations[0]["status"], "completed");
    assert_eq!(operations[1]["status"], "failed");
    assert_eq!(operations[2]["status"], "blocked");
    assert!(!untouched.join("source.bin.cake.json").exists());
}

#[test]
fn batch_merge_resume_preserves_completed_output_and_rejects_package_replacement() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let package = root.path().join("package");
    let rebuilt = root.path().join("rebuilt.bin");
    let state = root.path().join("merge.json");
    let spec = root.path().join("merge-job.json");
    fs::write(&source, b"merge resume fixture").unwrap();
    fs::create_dir(&package).unwrap();
    let split = cli()
        .args(["split"])
        .arg(&source)
        .args(["--slice-size", "4B", "--output-dir"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(split.status.success(), "{}", text(&split.stderr));
    let manifest = only_suffix(&package, ".cake.json");
    fs::write(
        &spec,
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 1,
            "name": "merge-resume",
            "failurePolicy": "stop",
            "operations": [{
                "id": "merge",
                "command": "merge",
                "manifest": manifest,
                "output": rebuilt
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let run = cli()
        .args(["batch", "run"])
        .arg(&spec)
        .args(["--state"])
        .arg(&state)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", text(&run.stderr));
    let before = fs::read(&rebuilt).unwrap();
    let resume = cli()
        .args(["batch", "resume"])
        .arg(&state)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(resume.status.success(), "{}", text(&resume.stderr));
    assert_eq!(json(&resume.stdout)["status"], "completed");
    assert_eq!(fs::read(&rebuilt).unwrap(), before);

    let replacement = root.path().join("replacement.cake.json");
    fs::copy(&manifest, &replacement).unwrap();
    fs::remove_file(&manifest).unwrap();
    fs::rename(&replacement, &manifest).unwrap();
    let rejected = cli()
        .args(["batch", "resume"])
        .arg(&state)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(9));
    assert_eq!(
        json(&rejected.stdout)["error"]["code"],
        "batch_completed_binding_changed"
    );
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

#[test]
fn sanitized_machine_contract_fixtures_preserve_stream_invariants() {
    let final_result: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/cli-contract/final-result.json"
    ))
    .unwrap();
    assert_eq!(final_result["schemaVersion"], 1);
    assert_eq!(final_result["command"], "inspect");

    let batch_result: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/cli-contract/batch-final-result.json"
    ))
    .unwrap();
    assert_eq!(batch_result["command"], "batch");
    assert!(batch_result["runId"].as_str().is_some());
    assert_eq!(batch_result["operationCounts"]["completed"], 1);

    for source in [
        include_str!("../../../tests/fixtures/cli-contract/stream.jsonl"),
        include_str!("../../../tests/fixtures/cli-contract/batch-failure.jsonl"),
        include_str!("../../../tests/fixtures/cli-contract/batch-cancelled.jsonl"),
    ] {
        let events = source
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert!(!events.is_empty());
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event["sequence"], (index + 1) as u64);
        }
        assert!(matches!(
            events.last().unwrap()["event"].as_str(),
            Some("completed" | "batch-completed" | "batch-failed" | "batch-cancelled")
        ));
        if events[0]["command"] == "batch" {
            let run_id = events[0]["runId"].clone();
            assert!(events.iter().all(|event| event["runId"] == run_id));
        }
    }
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
