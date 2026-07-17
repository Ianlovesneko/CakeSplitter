# Security Policy

## Supported versions

Security fixes are provided for the current v0.3.x source-release line. This
prototype has no long-term support commitment.

## Security boundaries

- Manifests, Slices, portable filenames, task metadata, and Worker messages are
  untrusted.
- Selected files are read as bytes and are never executed.
- The CLI does not invoke a shell to process file content.
- Manifest paths are portable names, never trusted absolute paths.
- Existing outputs must not be replaced.
- Human terminal output visibly escapes ASCII controls, ANSI controls, and
  Unicode bidirectional controls.
- The browser has no network fallback; production policy uses
  `connect-src 'none'`.
- The service worker caches only the marked, canonical application shell and
  declared same-origin static assets.
- OPFS stores bounded task metadata only; Clear All fences stale persistence.

The manifest parser rejects unknown fields and versions, malformed hashes,
unsafe or reserved filenames, invalid ranges, excessive nesting, excessive
size, and excessive Slice counts. Native Split revalidates its source across
processing and finalization. Native publication revalidates staged output
identity and content and uses an atomic no-replace platform primitive.

Direct Folder Mode remains disabled because current browser APIs do not expose
a portable atomic no-replace finalization operation. The generic secure-output
contract is tested, but no production browser adapter claims that unavailable
capability. Compatibility downloads remain bounded to 256 MiB.

SHA-256 proves byte equality with a manifest. It does not authenticate who
created the package. Cake Package 1.0 has no signature or authenticity layer,
and CakeSplitter is not a replacement for independent backups.

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

The sealed Phase 11 findings and their fixes are documented in
[`docs/v0.3-phase11-remediation-report.md`](docs/v0.3-phase11-remediation-report.md).
The final dependency, privacy, and accepted-risk assessment is in
[`docs/v0.3-security-report.md`](docs/v0.3-security-report.md).
