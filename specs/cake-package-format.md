# Cake Package Format 1.0

Status: implemented format for CakeSplitter v0.2.1. It is not an industry
standard, authenticated archive, or backup format.

## Package contents

A Cake Package contains one `.cake.json` manifest and zero or more numbered
`.slice` files. The original file is never modified.

```text
archive.tar.bin.001.slice
archive.tar.bin.002.slice
archive.tar.bin.003.slice
archive.tar.bin.cake.json
```

Indexes begin at one. Decimal width is the larger of three and the number of
digits in `sliceCount`, calculated before output begins.

## Manifest representation

The manifest is UTF-8 JSON described by
[`cake-manifest.schema.json`](./cake-manifest.schema.json). Writers should append
a newline. Required fields are:

- `format`: exactly `cakesplitter`;
- `version`: exactly `1.0`;
- `packageId`: UUID for the Split operation;
- `createdAt`: RFC 3339 timestamp;
- `original`: portable filename, exact size, lowercase SHA-256;
- `targetSliceSize`: requested maximum bytes per Slice, greater than zero;
- `sliceCount`: number of Slices; and
- `slices`: ordered rows of index, filename, offset, size, and SHA-256.

## Required validation

Readers must enforce JSON Schema plus these semantic rules:

1. Reject unknown fields, identifiers, and versions.
2. Require a table length equal to `sliceCount`.
3. Require unique, ordered, contiguous indexes beginning at one.
4. Require unique filenames exactly matching generated names.
5. Require contiguous ranges beginning at zero with no gaps or overlap.
6. Require each non-final Slice to equal `targetSliceSize` and the final Slice
   to equal the remainder.
7. Require ranges to cover `original.size` exactly.
8. Represent an empty Cake with zero Slices.
9. Require lowercase 64-character hexadecimal SHA-256 strings.
10. Require exact integers no larger than `9,007,199,254,740,991`.

## Resource limits

Conforming CakeSplitter v0.2.1 readers enforce:

| Limit | Value | Applies to |
|---|---:|---|
| Manifest UTF-8 size | 16 MiB | Rust and browser parser, before JSON decode |
| JSON nesting | 16 | Rust and browser parser, before JSON decode |
| Slice count/table | 50,000 | Format and both validators |
| Portable filename | 200 UTF-8 bytes | Original and Slice names |
| Exact integer | 9,007,199,254,740,991 | Sizes, offsets, indexes, counts |

Browser runtime limits are intentionally lower for operations: 10,000 selected
files, 256 MiB compatibility Split/Merge, and 1,000 Split downloads. Those are
runtime capability limits, not changes to Manifest 1.0.

## Portable filenames

Names are one UTF-8 path component. Reject empty names, `.` and `..`, separators,
drive/stream colons, `<`, `>`, `"`, `|`, `?`, `*`, control characters, leading
Unicode whitespace, trailing Unicode whitespace or `.`, and more than 200 UTF-8
bytes. Reject these Windows names case-insensitively, including any extension:

- `CON`, `PRN`, `AUX`, `NUL`, `CLOCK$`;
- `COM1` through `COM9`, including superscript `COM¹`, `COM²`, `COM³`; and
- `LPT1` through `LPT9`, including superscript `LPT¹`, `LPT²`, `LPT³`.

`COM0`, `COM10`, `LPT0`, `LPT10`, and ordinary names such as `console.bin` are
not reserved. Manifest paths never contain trusted absolute paths.

## Streaming and publication

Native implementations hash and copy with bounded buffers. They create new
`.partial` outputs exclusively, validate content and stable filesystem identity,
and publish only with an atomic no-replace platform operation. Existing output
must never be replaced. The rebuilt Cake is published only after its complete
SHA-256 equals `original.sha256`.

Browser v0.2.1 uses bounded downloads and does not claim atomic filesystem
publication. Before Merge, every selected Slice must have the exact expected
name, size, and SHA-256. Successful rebuilds in either runtime must have the same
SHA-256 as the original and as a rebuild in the other runtime.
