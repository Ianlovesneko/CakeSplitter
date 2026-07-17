use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::ExitCode,
};

use cakesplitter_core::{
    CancellationToken, SplitOptions, inspect_package, merge_package, split_file, verify_package,
};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "cakesplitter",
    version,
    about = "Split, inspect, verify, and rebuild local Cake Packages"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Split one file into verified .slice files and a .cake.json manifest.
    Split {
        file: PathBuf,
        #[arg(long, value_parser = parse_size)]
        slice_size: u64,
        #[arg(long)]
        output_dir: Option<PathBuf>,
    },
    /// Rebuild the original file and verify its final SHA-256 hash.
    Merge {
        manifest: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Read a manifest and report package completeness without hashing slices.
    Inspect { manifest: PathBuf },
    /// Verify every slice hash and package completeness.
    Verify { manifest: PathBuf },
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return render_clap_error(error),
    };
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = terminal_safe(&error.to_string());
            eprintln!("cakesplitter: [{}] {message}", error.code());
            let code = match error.code() {
                "cancelled" => 130,
                "invalid_json" | "invalid_manifest" | "invalid_slice_size" => 2,
                "missing_slices"
                | "unexpected_slices"
                | "corrupted_slices"
                | "final_hash_mismatch" => 3,
                "output_collision" => 4,
                _ => 1,
            };
            ExitCode::from(code)
        }
    }
}

fn render_clap_error(error: clap::Error) -> ExitCode {
    let exit_code = u8::try_from(error.exit_code()).unwrap_or(1);
    if error.use_stderr() {
        let message = terminal_safe(&error.to_string());
        eprint!("{message}");
    } else {
        print!("{error}");
    }
    ExitCode::from(exit_code)
}

fn terminal_safe(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());
    for character in text.chars() {
        if is_bidi_control(character) {
            write!(&mut safe, "\\u{{{:x}}}", character as u32)
                .expect("writing to a String cannot fail");
        } else if character.is_control() {
            safe.extend(character.escape_default());
        } else {
            safe.push(character);
        }
    }
    safe
}

fn json_terminal_safe(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());
    for character in text.chars() {
        if is_bidi_control(character) {
            write!(&mut safe, "\\u{:04x}", character as u32)
                .expect("writing to a String cannot fail");
        } else {
            safe.push(character);
        }
    }
    safe
}

fn terminal_path(path: &Path) -> String {
    terminal_safe(&path.display().to_string())
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

fn run(cli: Cli) -> Result<(), cakesplitter_core::CoreError> {
    let cancellation = CancellationToken::new();
    match cli.command {
        Command::Split {
            file,
            slice_size,
            output_dir,
        } => {
            let output_dir = output_dir.unwrap_or_else(|| {
                file.parent()
                    .map_or_else(|| PathBuf::from("."), PathBuf::from)
            });
            let manifest = split_file(
                &file,
                &SplitOptions {
                    slice_size,
                    output_dir,
                    cancellation,
                },
            )?;
            println!("Created {}", terminal_path(&manifest));
        }
        Command::Merge { manifest, output } => {
            merge_package(&manifest, &output, &cancellation)?;
            println!("Rebuilt and verified {}", terminal_path(&output));
        }
        Command::Inspect { manifest } => {
            let inspection = inspect_package(&manifest, false, &cancellation)?;
            let serialized = serde_json::to_string_pretty(&inspection)
                .expect("inspection serialization cannot fail");
            println!("{}", json_terminal_safe(&serialized));
        }
        Command::Verify { manifest } => {
            let inspection = verify_package(&manifest, &cancellation)?;
            if !inspection.verified {
                if !inspection.missing.is_empty() {
                    return Err(cakesplitter_core::CoreError::MissingSlices(
                        inspection.missing,
                    ));
                }
                if !inspection.corrupted.is_empty() {
                    return Err(cakesplitter_core::CoreError::CorruptedSlices(
                        inspection.corrupted,
                    ));
                }
                return Err(cakesplitter_core::CoreError::UnexpectedSlices(
                    inspection.unexpected,
                ));
            }
            println!("Verified {} slices", inspection.found_slice_count);
        }
    }
    Ok(())
}

fn parse_size(value: &str) -> Result<u64, String> {
    let normalized = value.trim().to_ascii_lowercase().replace('_', "");
    let split_at = normalized
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(normalized.len());
    let (number, unit) = normalized.split_at(split_at);
    let number = number
        .parse::<u64>()
        .map_err(|_| format!("invalid size: {}", terminal_safe(value)))?;
    let multiplier = match unit.trim() {
        "" | "b" => 1,
        "k" | "kb" => 1_000,
        "m" | "mb" => 1_000_000,
        "g" | "gb" => 1_000_000_000,
        "ki" | "kib" => 1_024,
        "mi" | "mib" => 1_048_576,
        "gi" | "gib" => 1_073_741_824,
        _ => return Err(format!("unsupported size unit: {}", terminal_safe(unit))),
    };
    number
        .checked_mul(multiplier)
        .filter(|size| *size > 0)
        .ok_or_else(|| "size must be greater than zero and within range".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{parse_size, terminal_safe};

    #[test]
    fn parses_common_units() {
        assert_eq!(parse_size("10"), Ok(10));
        assert_eq!(parse_size("2 MiB"), Ok(2_097_152));
        assert_eq!(parse_size("3gb"), Ok(3_000_000_000));
        assert!(parse_size("0").is_err());
    }

    #[test]
    fn escapes_terminal_controls_without_hiding_unicode() {
        let safe = terminal_safe("生日蛋糕\u{1b}\r\n");
        assert_eq!(safe, "生日蛋糕\\u{1b}\\r\\n");
        assert!(!safe.chars().any(char::is_control));
    }
}
