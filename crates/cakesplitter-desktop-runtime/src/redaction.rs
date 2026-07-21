use std::{path::Path, sync::OnceLock};

use regex::{Captures, Regex};

const MAX_REDACTED_TEXT_CHARS: usize = 16_384;

pub fn redact_text(value: &str) -> String {
    let safe_controls = value
        .chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                '�'
            } else {
                character
            }
        })
        .take(MAX_REDACTED_TEXT_CHARS)
        .collect::<String>();
    let value = credential_url_pattern()
        .replace_all(&safe_controls, "<credentials-url>")
        .into_owned();
    let mut urls = Vec::new();
    let protected_urls = url_pattern()
        .replace_all(&value, |captures: &Captures<'_>| {
            let index = urls.len();
            urls.push(captures[0].to_owned());
            format!("<cakesplitter-url-{index}>")
        })
        .into_owned();
    let value = email_pattern().replace_all(&protected_urls, "<email>");
    let value = secret_pattern().replace_all(&value, "${1}=<redacted>");
    let value = environment_pattern().replace_all(&value, "<environment-value>");
    let mut value = redact_windows_paths(&value);
    for (index, url) in urls.into_iter().enumerate() {
        value = value.replace(&format!("<cakesplitter-url-{index}>"), &url);
    }
    value
}

pub fn sanitize_label(value: &str) -> String {
    redact_text(value).chars().take(500).collect()
}

pub fn masked_path(path: &Path, include_path_detail: bool) -> String {
    if include_path_detail {
        return path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("<local-path>\\{}", sanitize_label(name)))
            .unwrap_or_else(|| "<selected-path>".to_owned());
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("…\\{}", sanitize_label(name)))
        .unwrap_or_else(|| "<selected-path>".to_owned())
}

fn url_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"(?i)https?://[^\s,;]+").expect("valid URL pattern"))
}

fn redact_windows_paths(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut last = 0;
    let mut index = 0;
    while index < bytes.len() {
        let drive = index + 2 < bytes.len()
            && bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && matches!(bytes[index + 2], b'\\' | b'/');
        let unc = index + 2 < bytes.len()
            && bytes[index] == b'\\'
            && bytes[index + 1] == b'\\'
            && bytes[index + 2] != b'<'
            && bytes[index + 2] != b'\\';
        if !(drive || unc) || (index > 0 && is_word_byte(bytes[index - 1])) {
            index += 1;
            continue;
        }
        let end = scan_path_end(bytes, index + if drive { 3 } else { 2 });
        if end <= index + if drive { 3 } else { 2 } {
            index += 1;
            continue;
        }
        output.push_str(&value[last..index]);
        let path = &value[index..end];
        let lower = path.to_ascii_lowercase();
        output.push_str(
            if lower.contains("\\users\\") || lower.contains("/users/") {
                "<user-profile>"
            } else {
                "<local-path>"
            },
        );
        last = end;
        index = end;
    }
    output.push_str(&value[last..]);
    output
}

fn scan_path_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() {
        match bytes[index] {
            b'\r' | b'\n' | b',' | b';' | b':' | b'<' => break,
            b' ' | b'\t' if bytes.get(index + 1) == Some(&b'<') => break,
            _ => index += 1,
        }
    }
    index
}

fn is_word_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'_'
}

fn credential_url_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(https?://)[^/\s:@]+:[^/\s@]+@")
            .expect("valid credential URL redaction pattern")
    })
}

fn email_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}")
            .expect("valid email redaction pattern")
    })
}

fn secret_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\b(api[_-]?key|token|secret|password|authorization)\b\s*[:=]\s*[^\s,;]+")
            .expect("valid secret redaction pattern")
    })
}

fn environment_pattern() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)(%[A-Z_][A-Z0-9_]*%|\$env:[A-Z_][A-Z0-9_]*)")
            .expect("valid environment redaction pattern")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_redaction_masks_paths_emails_credentials_secrets_and_controls() {
        let value = concat!(
            "C:\\Users\\Private Name\\secret.bin ",
            "person@example.test ",
            "https://user:password@example.test/path ",
            "api_key=super-secret ",
            "%LOCALAPPDATA% ",
            "label\u{1b}[31m"
        );
        let redacted = redact_text(value);
        for forbidden in [
            "Private Name",
            "D:\\Private Name",
            "server\\sensitive-share",
            "person@example.test",
            "user:password",
            "super-secret",
            "LOCALAPPDATA",
            "\u{1b}",
        ] {
            assert!(!redacted.contains(forbidden));
        }
        assert!(redacted.contains("<user-profile>"));
        assert!(redacted.contains("<email>"));
        assert!(redacted.contains("api_key=<redacted>"));
    }

    #[test]
    fn shared_redaction_masks_drive_and_unc_paths_without_matching_urls() {
        let value = concat!(
            r"D:\Private Name\client\source.bin ",
            r"\\server\sensitive-share\client\manifest.cake.json ",
            "https://example.test/path"
        );
        let redacted = redact_text(value);
        assert!(!redacted.contains("D:\\Private"));
        assert!(!redacted.contains("server\\sensitive-share"));
        assert!(redacted.contains("https://example.test/path"));
    }

    #[test]
    fn paths_are_filename_only_unless_detail_is_explicitly_requested() {
        let path = Path::new(r"C:\Users\Private Name\package\sample.bin");
        assert_eq!(masked_path(path, false), r"…\sample.bin");
        let detailed = masked_path(path, true);
        assert!(!detailed.contains("Private Name"));
        assert!(detailed.contains("sample.bin"));
    }
}
