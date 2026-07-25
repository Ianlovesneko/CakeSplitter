# Security Policy

## Supported versions

Security fixes are provided for the current `v0.8.1` public pre-release. This
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

Do not disclose an unpatched vulnerability through a public issue, discussion,
pull request, or other public channel. Use
[GitHub Private Vulnerability Reporting](https://github.com/Ianlovesneko/CakeSplitter/security/advisories/new).

Include the affected CakeSplitter version or commit, operating system and
browser/runtime, required reproduction conditions, realistic impact, and the
smallest safe evidence that demonstrates the issue. Synthetic fixtures are
preferred. Do not upload real sensitive Cake Packages, private user files,
credentials, tokens, or unredacted private paths.

Ordinary non-security bugs can use the repository's bug-report template.

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
[`docs/v0.6.0-security-report.md`](docs/v0.6.0-security-report.md). The current
public pre-release review is in
[`docs/v0.8.1-security-report.md`](docs/v0.8.1-security-report.md). The
development-tool paths affected in v0.8.0 by
[`GHSA-mh99-v99m-4gvg`](https://github.com/advisories/GHSA-mh99-v99m-4gvg)
were not present in the shipped runtime graph and are refreshed in v0.8.1.

The public classification of historical material is tracked in
[`docs/public-security-report-index.md`](docs/public-security-report-index.md).
