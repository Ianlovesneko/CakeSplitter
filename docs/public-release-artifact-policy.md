# Public Release Artifact Policy

The v0.7.0 result is source-only local publication-candidate evidence. No
installer, executable, Web archive, or checksum bundle is published or
committed. Private preview artifacts remain outside Git with their own SHA-256
manifests.

Before v0.8, each proposed artifact must record version, platform, architecture,
size, SHA-256, signing state, license/notice inclusion, and validation state.
Unsigned Windows artifacts must disclose SmartScreen implications and supported
platform boundaries. Do not claim public downloads, signatures, or support for
macOS/Linux until those artifacts are independently validated.
