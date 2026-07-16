# Security Policy

## Supported versions

Security fixes are provided for the current v0.2.x release line. This
prototype has no long-term support commitment.

## Security boundaries

- Manifests, Slices, portable filenames, and Worker messages are untrusted.
- Selected files are read as bytes and are never executed.
- The CLI does not invoke a shell to process file content.
- Manifest paths are portable names, never trusted absolute paths.
- Existing outputs must not be replaced.
- The browser has no network fallback; production policy uses
  `connect-src 'none'`.

The manifest parser rejects unknown fields and versions, malformed hashes,
unsafe or reserved filenames, invalid ranges, excessive nesting, excessive
size, and excessive Slice counts. Native publication revalidates staged file
identity and content and uses an atomic no-replace platform primitive. Browser
direct-folder publication is disabled in v0.2.1; bounded downloads are used
instead. Every Worker request and response is validated at runtime.

SHA-256 proves byte equality with the manifest. It does not authenticate who
created a package. CakeSplitter is not a replacement for independent backups.

## Reporting a vulnerability

Use the repository host's private security-advisory feature. Include the
affected version, reproduction steps, impact, operating system/browser, and the
smallest non-sensitive package that demonstrates the issue. Do not attach real
private files or manifests containing sensitive filenames.

Do not open a public issue for an unpatched vulnerability. Ordinary bugs can use
the repository's bug-report template.

## Disclosure and response

Maintainers should acknowledge a private report, validate the claimed dataflow,
assign severity only after validation, and coordinate a fix and disclosure with
the reporter. Scanner output alone is not treated as a confirmed vulnerability.

The v0.2.1 review and accepted risks are recorded in
[`docs/v0.2-security-report.md`](docs/v0.2-security-report.md).
