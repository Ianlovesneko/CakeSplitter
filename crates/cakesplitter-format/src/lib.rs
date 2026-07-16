//! Portable, versioned Cake Package manifest types and strict validation.

use std::collections::HashSet;

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const FORMAT_IDENTIFIER: &str = "cakesplitter";
pub const FORMAT_VERSION: &str = "1.0";
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_SLICE_COUNT: u64 = 50_000;
pub const MAX_FILENAME_BYTES: usize = 200;
pub const MAX_JSON_NESTING: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CakeManifest {
    pub format: String,
    pub version: String,
    pub package_id: String,
    pub created_at: String,
    pub original: OriginalFile,
    pub target_slice_size: u64,
    pub slice_count: u64,
    pub slices: Vec<SliceEntry>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OriginalFile {
    pub filename: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SliceEntry {
    pub index: u64,
    pub filename: String,
    pub offset: u64,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ManifestError {
    #[error("manifest exceeds the {MAX_MANIFEST_BYTES}-byte limit")]
    ManifestTooLarge,
    #[error("manifest JSON nesting exceeds the maximum depth of {MAX_JSON_NESTING}")]
    JsonNestingTooDeep,
    #[error("manifest is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("unsupported format identifier: {0}")]
    UnsupportedFormat(String),
    #[error("unsupported format version: {0}")]
    UnsupportedVersion(String),
    #[error("invalid package ID")]
    InvalidPackageId,
    #[error("invalid creation timestamp")]
    InvalidTimestamp,
    #[error("unsafe or invalid portable filename: {0}")]
    UnsafeFilename(String),
    #[error("portable filename is {actual} UTF-8 bytes; maximum is {maximum}")]
    FilenameTooLong { actual: usize, maximum: usize },
    #[error("invalid SHA-256 value for {0}")]
    InvalidHash(String),
    #[error("numeric value exceeds the cross-runtime safe integer range")]
    UnsafeInteger,
    #[error("target slice size must be greater than zero")]
    InvalidTargetSliceSize,
    #[error("slice count does not match the original size and target slice size")]
    InvalidSliceCount,
    #[error("slice count exceeds the supported maximum of {MAX_SLICE_COUNT}")]
    TooManySlices,
    #[error("slice table length does not match sliceCount")]
    SliceTableLength,
    #[error("duplicate slice index: {0}")]
    DuplicateIndex(u64),
    #[error("duplicate slice filename: {0}")]
    DuplicateFilename(String),
    #[error("slice indexes must be ordered and contiguous; expected {expected}, found {actual}")]
    InvalidIndex { expected: u64, actual: u64 },
    #[error("slice offset is invalid at index {index}; expected {expected}, found {actual}")]
    InvalidOffset {
        index: u64,
        expected: u64,
        actual: u64,
    },
    #[error("slice size is invalid at index {index}; expected {expected}, found {actual}")]
    InvalidSliceSize {
        index: u64,
        expected: u64,
        actual: u64,
    },
    #[error("slice filename is invalid at index {index}; expected {expected}, found {actual}")]
    InvalidSliceFilename {
        index: u64,
        expected: String,
        actual: String,
    },
    #[error("slice ranges do not exactly cover the original file")]
    IncompleteCoverage,
}

impl CakeManifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.format != FORMAT_IDENTIFIER {
            return Err(ManifestError::UnsupportedFormat(self.format.clone()));
        }
        if self.version != FORMAT_VERSION {
            return Err(ManifestError::UnsupportedVersion(self.version.clone()));
        }
        if Uuid::parse_str(&self.package_id).is_err() {
            return Err(ManifestError::InvalidPackageId);
        }
        if DateTime::parse_from_rfc3339(&self.created_at).is_err() {
            return Err(ManifestError::InvalidTimestamp);
        }
        validate_portable_filename(&self.original.filename)?;
        validate_hash("original file", &self.original.sha256)?;
        if self.original.size > MAX_SAFE_INTEGER
            || self.target_slice_size > MAX_SAFE_INTEGER
            || self.slice_count > MAX_SAFE_INTEGER
        {
            return Err(ManifestError::UnsafeInteger);
        }
        if self.target_slice_size == 0 {
            return Err(ManifestError::InvalidTargetSliceSize);
        }
        if self.slice_count > MAX_SLICE_COUNT || self.slices.len() > MAX_SLICE_COUNT as usize {
            return Err(ManifestError::TooManySlices);
        }

        let expected_count = expected_slice_count(self.original.size, self.target_slice_size);
        if self.slice_count != expected_count {
            return Err(ManifestError::InvalidSliceCount);
        }
        if self.slices.len() as u64 != self.slice_count {
            return Err(ManifestError::SliceTableLength);
        }

        let width = slice_index_width(self.slice_count);
        let mut expected_offset = 0_u64;
        let mut indexes = HashSet::new();
        let mut filenames = HashSet::new();
        for (position, slice) in self.slices.iter().enumerate() {
            if slice.index > MAX_SAFE_INTEGER
                || slice.offset > MAX_SAFE_INTEGER
                || slice.size > MAX_SAFE_INTEGER
            {
                return Err(ManifestError::UnsafeInteger);
            }
            if !indexes.insert(slice.index) {
                return Err(ManifestError::DuplicateIndex(slice.index));
            }
            if !filenames.insert(slice.filename.clone()) {
                return Err(ManifestError::DuplicateFilename(slice.filename.clone()));
            }
            let expected_index = position as u64 + 1;
            if slice.index != expected_index {
                return Err(ManifestError::InvalidIndex {
                    expected: expected_index,
                    actual: slice.index,
                });
            }
            validate_portable_filename(&slice.filename)?;
            validate_hash(&format!("slice {}", slice.index), &slice.sha256)?;
            if slice.offset != expected_offset {
                return Err(ManifestError::InvalidOffset {
                    index: slice.index,
                    expected: expected_offset,
                    actual: slice.offset,
                });
            }
            let remaining = self.original.size.saturating_sub(expected_offset);
            let expected_size = remaining.min(self.target_slice_size);
            if slice.size != expected_size {
                return Err(ManifestError::InvalidSliceSize {
                    index: slice.index,
                    expected: expected_size,
                    actual: slice.size,
                });
            }
            let expected_filename = slice_filename(&self.original.filename, slice.index, width);
            if slice.filename != expected_filename {
                return Err(ManifestError::InvalidSliceFilename {
                    index: slice.index,
                    expected: expected_filename,
                    actual: slice.filename.clone(),
                });
            }
            expected_offset = expected_offset
                .checked_add(slice.size)
                .ok_or(ManifestError::UnsafeInteger)?;
        }
        if expected_offset != self.original.size {
            return Err(ManifestError::IncompleteCoverage);
        }
        Ok(())
    }

    pub fn manifest_filename(&self) -> String {
        format!("{}.cake.json", self.original.filename)
    }
}

pub fn parse_manifest_json(json: &str) -> Result<CakeManifest, ManifestError> {
    if json.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::ManifestTooLarge);
    }
    validate_json_nesting(json)?;
    let manifest: CakeManifest = serde_json::from_str(json)
        .map_err(|error| ManifestError::InvalidJson(error.to_string()))?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn expected_slice_count(size: u64, target_slice_size: u64) -> u64 {
    if size == 0 || target_slice_size == 0 {
        0
    } else {
        1 + (size - 1) / target_slice_size
    }
}

pub fn slice_index_width(slice_count: u64) -> usize {
    3_usize.max(slice_count.max(1).to_string().len())
}

pub fn slice_filename(original_filename: &str, index: u64, width: usize) -> String {
    format!("{original_filename}.{index:0width$}.slice")
}

pub fn validate_portable_filename(filename: &str) -> Result<(), ManifestError> {
    let byte_length = filename.len();
    if byte_length > MAX_FILENAME_BYTES {
        return Err(ManifestError::FilenameTooLong {
            actual: byte_length,
            maximum: MAX_FILENAME_BYTES,
        });
    }
    let first = filename.chars().next();
    let last = filename.chars().next_back();
    let invalid = filename.is_empty()
        || filename == "."
        || filename == ".."
        || filename.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
        || filename.contains('\0')
        || filename.chars().any(char::is_control)
        || first.is_some_and(char::is_whitespace)
        || last.is_some_and(|character| character == '.' || character.is_whitespace())
        || filename.contains("..\\")
        || filename.contains("../")
        || is_windows_reserved_name(filename);
    if invalid {
        return Err(ManifestError::UnsafeFilename(filename.to_owned()));
    }
    Ok(())
}

fn is_windows_reserved_name(filename: &str) -> bool {
    let basename = filename
        .split('.')
        .next()
        .unwrap_or_default()
        .chars()
        .flat_map(char::to_uppercase)
        .collect::<String>();
    if matches!(basename.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$") {
        return true;
    }
    if matches!(
        basename.as_str(),
        "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
    ) {
        return true;
    }
    ["COM", "LPT"].iter().any(|prefix| {
        basename.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
    })
}

fn validate_json_nesting(json: &str) -> Result<(), ManifestError> {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in json.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_JSON_NESTING {
                    return Err(ManifestError::JsonNestingTooDeep);
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

fn validate_hash(label: &str, value: &str) -> Result<(), ManifestError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ManifestError::InvalidHash(label.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> CakeManifest {
        CakeManifest {
            format: FORMAT_IDENTIFIER.to_owned(),
            version: FORMAT_VERSION.to_owned(),
            package_id: "b5c7a2ac-1d0f-44b6-a1d6-0f9f21983f8f".to_owned(),
            created_at: "2026-07-16T04:00:00Z".to_owned(),
            original: OriginalFile {
                filename: "archive.tar.bin".to_owned(),
                size: 5,
                sha256: "a".repeat(64),
            },
            target_slice_size: 3,
            slice_count: 2,
            slices: vec![
                SliceEntry {
                    index: 1,
                    filename: "archive.tar.bin.001.slice".to_owned(),
                    offset: 0,
                    size: 3,
                    sha256: "b".repeat(64),
                },
                SliceEntry {
                    index: 2,
                    filename: "archive.tar.bin.002.slice".to_owned(),
                    offset: 3,
                    size: 2,
                    sha256: "c".repeat(64),
                },
            ],
        }
    }

    #[test]
    fn accepts_valid_manifest() {
        assert_eq!(valid_manifest().validate(), Ok(()));
    }

    #[test]
    fn rejects_path_traversal() {
        let mut manifest = valid_manifest();
        manifest.original.filename = "../secret.bin".to_owned();
        assert!(matches!(
            manifest.validate(),
            Err(ManifestError::UnsafeFilename(_))
        ));
    }

    #[test]
    fn rejects_duplicate_and_wrong_order_indexes() {
        let mut duplicate = valid_manifest();
        duplicate.slices[1].index = 1;
        assert!(matches!(
            duplicate.validate(),
            Err(ManifestError::DuplicateIndex(1))
        ));

        let mut wrong_order = valid_manifest();
        wrong_order.slices.swap(0, 1);
        assert!(matches!(
            wrong_order.validate(),
            Err(ManifestError::InvalidIndex { .. })
        ));
    }

    #[test]
    fn rejects_overlaps_impossible_sizes_versions_and_hashes() {
        let mut overlap = valid_manifest();
        overlap.slices[1].offset = 2;
        assert!(matches!(
            overlap.validate(),
            Err(ManifestError::InvalidOffset { .. })
        ));

        let mut size = valid_manifest();
        size.slices[1].size = 3;
        assert!(matches!(
            size.validate(),
            Err(ManifestError::InvalidSliceSize { .. })
        ));

        let mut version = valid_manifest();
        version.version = "2.0".to_owned();
        assert!(matches!(
            version.validate(),
            Err(ManifestError::UnsupportedVersion(_))
        ));

        let mut hash = valid_manifest();
        hash.original.sha256 = "ABC".to_owned();
        assert!(matches!(
            hash.validate(),
            Err(ManifestError::InvalidHash(_))
        ));
    }

    #[test]
    fn rejects_unknown_json_fields() {
        let json = r#"{"format":"cakesplitter","version":"1.0","extra":true}"#;
        assert!(serde_json::from_str::<CakeManifest>(json).is_err());
    }

    #[test]
    fn enforces_manifest_resource_limits_before_validation() {
        assert!(matches!(
            parse_manifest_json(&" ".repeat(MAX_MANIFEST_BYTES + 1)),
            Err(ManifestError::ManifestTooLarge)
        ));
        let deeply_nested = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_NESTING + 1),
            "]".repeat(MAX_JSON_NESTING + 1)
        );
        assert!(matches!(
            parse_manifest_json(&deeply_nested),
            Err(ManifestError::JsonNestingTooDeep)
        ));

        let mut manifest = valid_manifest();
        manifest.slice_count = MAX_SLICE_COUNT + 1;
        assert!(matches!(
            manifest.validate(),
            Err(ManifestError::TooManySlices)
        ));
    }

    #[test]
    fn rejects_reserved_invalid_and_overlong_portable_names() {
        for filename in [
            "CON",
            "con.txt",
            "COM1.bin",
            "LPT9",
            "COM¹.log",
            "bad<name.bin",
            "bad|name.bin",
            " leading.bin",
            "trailing\u{2003}",
        ] {
            assert!(
                validate_portable_filename(filename).is_err(),
                "{filename:?} was unexpectedly accepted"
            );
        }
        assert!(matches!(
            validate_portable_filename(&"a".repeat(MAX_FILENAME_BYTES + 1)),
            Err(ManifestError::FilenameTooLong { .. })
        ));
        for filename in ["console.bin", "COM0.bin", "LPT10.bin", "生日蛋糕.bin"] {
            assert_eq!(validate_portable_filename(filename), Ok(()));
        }
    }
}
