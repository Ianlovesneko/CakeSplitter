use std::{fs, path::Path, process::Command, time::SystemTime};

use tempfile::tempdir;

const BIDI_CONTROLS: [char; 12] = [
    '\u{061c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}',
    '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
];

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

#[test]
fn every_bidi_control_is_visible_on_split_stdout() {
    let directory = tempdir().unwrap();
    let output_dir = directory.path().join("packages");
    fs::create_dir_all(&output_dir).unwrap();

    for (index, control) in BIDI_CONTROLS.into_iter().enumerate() {
        let input = directory
            .path()
            .join(format!("invoice-{index}{control}gpj.txt"));
        fs::write(&input, b"terminal safety").unwrap();
        let output = cli()
            .arg("split")
            .arg(&input)
            .arg("--slice-size")
            .arg("64")
            .arg("--output-dir")
            .arg(&output_dir)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let rendered = terminal_output(&output.stdout);
        assert!(!rendered.contains(control));
        assert!(rendered.contains(&format!("\\u{{{:x}}}", control as u32)));
    }
}

#[test]
fn split_merge_inspect_verify_and_errors_neutralize_untrusted_paths() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("invoice\u{202e}gpj.txt");
    let output_dir = directory.path().join("package");
    fs::create_dir_all(&output_dir).unwrap();
    fs::write(&input, b"verified terminal flow").unwrap();

    let split = cli()
        .arg("split")
        .arg(&input)
        .arg("--slice-size")
        .arg("64")
        .arg("--output-dir")
        .arg(&output_dir)
        .output()
        .unwrap();
    assert!(
        split.status.success(),
        "{}",
        String::from_utf8_lossy(&split.stderr)
    );
    assert_safe_human_output(&split.stdout, '\u{202e}');

    let manifest = only_file_with_suffix(&output_dir, ".cake.json");
    let inspect = cli().arg("inspect").arg(&manifest).output().unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let inspect_text = terminal_output(&inspect.stdout);
    assert!(!inspect_text.contains('\u{202e}'));
    assert!(inspect_text.contains("\\u202e"));
    let parsed: serde_json::Value = serde_json::from_str(&inspect_text).unwrap();
    assert_eq!(
        parsed["manifest"]["original"]["filename"],
        "invoice\u{202e}gpj.txt"
    );

    let verify = cli().arg("verify").arg(&manifest).output().unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    assert!(!terminal_output(&verify.stdout).chars().any(is_bidi_control));

    let rebuilt = directory.path().join("rebuilt\u{202d}txt.bin");
    let merge = cli()
        .arg("merge")
        .arg(&manifest)
        .arg("--output")
        .arg(&rebuilt)
        .output()
        .unwrap();
    assert!(
        merge.status.success(),
        "{}",
        String::from_utf8_lossy(&merge.stderr)
    );
    assert_safe_human_output(&merge.stdout, '\u{202d}');

    let slice = only_file_with_suffix(&output_dir, ".slice");
    fs::remove_file(slice).unwrap();
    let failed_verify = cli().arg("verify").arg(&manifest).output().unwrap();
    assert_eq!(failed_verify.status.code(), Some(3));
    assert_safe_human_output(&failed_verify.stderr, '\u{202e}');
}

#[test]
fn bidi_plus_ansi_is_escaped_in_clap_errors() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("safe.bin");
    fs::write(&input, b"safe").unwrap();
    let output = cli()
        .arg("split")
        .arg(&input)
        .arg("--slice-size")
        .arg("1\u{202e}\u{1b}")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let rendered = terminal_output(&output.stderr);
    assert!(!rendered.contains('\u{202e}'));
    assert!(!rendered.contains('\u{1b}'));
    assert!(rendered.contains("\\u{202e}"));
    assert!(rendered.contains("\\u{1b}"));
}

#[test]
fn safe_unicode_filename_remains_readable() {
    let directory = tempdir().unwrap();
    let input = directory.path().join("生日蛋糕.bin");
    fs::write(&input, b"unicode").unwrap();
    let output = cli()
        .arg("split")
        .arg(&input)
        .arg("--slice-size")
        .arg("64")
        .arg("--output-dir")
        .arg(directory.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(terminal_output(&output.stdout).contains("生日蛋糕.bin.cake.json"));
}

fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cakesplitter"))
}

fn terminal_output(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

fn assert_safe_human_output(bytes: &[u8], control: char) {
    let rendered = terminal_output(bytes);
    assert!(!rendered.contains(control));
    assert!(rendered.contains(&format!("\\u{{{:x}}}", control as u32)));
    assert!(!rendered.chars().any(is_bidi_control));
    assert!(!rendered.chars().any(char::is_control));
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn only_file_with_suffix(directory: &Path, suffix: &str) -> std::path::PathBuf {
    let matches = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(suffix)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        matches.len(),
        1,
        "expected one {suffix} file in {directory:?}"
    );
    matches.into_iter().next().unwrap()
}
