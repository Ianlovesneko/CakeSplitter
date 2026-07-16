use std::{fs, process::Command, time::SystemTime};

#[test]
fn rejected_manifest_controls_are_escaped_on_stderr() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "cakesplitter-terminal-test-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let manifest = directory.join("unsafe.cake.json");
    fs::write(
        &manifest,
        r#"{
  "format": "cakesplitter\u001b",
  "version": "1.0",
  "packageId": "b5c7a2ac-1d0f-44b6-a1d6-0f9f21983f8f",
  "createdAt": "2026-07-16T04:00:00Z",
  "original": { "filename": "safe.bin", "size": 0, "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
  "targetSliceSize": 1,
  "sliceCount": 0,
  "slices": []
}"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cakesplitter"))
        .arg("inspect")
        .arg(&manifest)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&directory);

    assert_eq!(output.status.code(), Some(2));
    assert!(!output.stderr.contains(&0x1b));
    let rendered = String::from_utf8(output.stderr).unwrap();
    assert!(rendered.contains("\\u{1b}"));
    let content = rendered.strip_suffix('\n').unwrap_or(&rendered);
    assert!(!content.chars().any(char::is_control));
}
