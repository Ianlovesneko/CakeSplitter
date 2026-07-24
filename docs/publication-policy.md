# CakeSplitter Release and Publication Policy

## Current status

CakeSplitter `v0.8.0` is the first public GitHub pre-release. It remains
early-stage software and is not a stable or long-term-support release.

Versions v0.4 through v0.6 remain preserved local development releases. v0.7
is the completed public-release hardening and history-audit checkpoint. Public
history preserves the validated local commit and tag lineage.

## Publication sequence

- v0.4 through v0.6: local development releases;
- v0.7: public-release hardening and repository-history audit;
- v0.8: first GitHub public pre-release; and
- v1.0: stable public release.

## Public-release requirements

Each public release requires:

- a repository-history secret, identity, path, and generated-file review;
- license and third-party-notice review;
- focused security review and dependency audits;
- signed release-tag verification;
- clean source and public fresh-clone validation;
- validated release artifacts and SHA-256 checksums;
- accurate unsigned-binary and SmartScreen disclosures; and
- verified GitHub security reporting and branch-protection settings.

Preserve complete Git history unless a validated sensitive-history issue
genuinely requires a separately reviewed rewrite. Never force-push a release
branch or recreate an existing release tag.
