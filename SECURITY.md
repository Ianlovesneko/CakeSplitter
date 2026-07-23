# Security Policy

## Supported versions

Security fixes are provided for the current v0.5.x local source-release line. This
early prototype has no long-term support commitment.

## Security boundaries

- Manifests, Slices, portable filenames, task metadata, browser Worker
  messages, renderer IPC arguments, and recovered state are untrusted.
- Selected files are read as bytes and are never executed.
- CakeSplitter does not invoke a shell or arbitrary process for file work.
- Manifest paths are portable names, never trusted absolute paths.
- Existing outputs must not be replaced.
- Human terminal output visibly escapes ASCII controls, ANSI controls, and
  Unicode bidirectional controls.
- Desktop filesystem access is available only through narrowly scoped Rust
  commands and short-lived selection tokens.
- Desktop source, package, selection, and destination identities are checked
  before use and across recovery; ambiguous filesystems fail closed.
- Desktop task admission, recovery, history, metadata, and package enumeration
  are bounded in Rust, not only in the renderer.
- Desktop has no HTTP, updater, shell, process, telemetry, or unrestricted
  filesystem capability and loads no remote application content.
- The Web app has no network fallback; production policy uses
  `connect-src 'none'`.
- The Web service worker caches only the marked canonical application shell and
  declared same-origin static assets.
- Browser OPFS stores bounded task metadata only; Clear All fences stale
  persistence.

The manifest parser rejects unknown fields and versions, malformed hashes,
unsafe or reserved filenames, invalid ranges, excessive nesting, excessive
size, and excessive Slice counts. Native publication retains Windows directory
authority, revalidates object identity and content at security-sensitive
boundaries, and uses atomic no-replace platform primitives.

Browser Direct Folder Mode remains disabled because current browser APIs do not
expose a portable atomic no-replace finalization operation. Compatibility
downloads remain bounded to 256 MiB and do not imply unlimited browser support.

SHA-256 proves byte equality with a manifest. It does not authenticate who
created the package. Cake Package 1.0 has no signature, encryption, or
authenticity layer, and CakeSplitter is not a replacement for independent
backups.

## Reporting a vulnerability

Use the repository host's private security-advisory feature. Include the
affected version, reproduction steps, realistic impact, operating
system/browser, and the smallest non-sensitive package that demonstrates the
issue. Do not attach real private files or manifests with sensitive filenames.

Do not open a public issue for an unpatched vulnerability. Ordinary bugs can
use the repository's bug-report template.

## Disclosure and response

Maintainers should acknowledge a private report, validate the source-to-sink
path, assign severity only after validation, and coordinate a fix and
disclosure with the reporter. Scanner output alone is not a confirmed
vulnerability.

The v0.4 findings and focused remediations are documented in
[`docs/v0.4-medium-remediation-report.md`](docs/v0.4-medium-remediation-report.md)
and [`docs/v0.4-low-remediation-report.md`](docs/v0.4-low-remediation-report.md).
The final dependency, privacy, packaged-runtime, and accepted-risk assessment
is in [`docs/v0.4-security-report.md`](docs/v0.4-security-report.md).
The focused v0.5 security review and accepted-risk assessment is in
[`docs/v0.6.0-security-report.md`](docs/v0.6.0-security-report.md). v0.6.0 is a
private local release; public publication is planned for v0.8.0.
