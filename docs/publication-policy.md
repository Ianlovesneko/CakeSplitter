# CakeSplitter Release and Publication Policy

## Current status

The repository is currently a private `v0.7.0-dev` public-hardening
checkpoint. No remote, public download, or GitHub release exists.

CakeSplitter v0.4.0 is a completed local release. Its release commit and
annotated tag are preserved in the local Git repository, and no Git remote is
required for this release.

The v0.4.0 Windows installer is not publicly distributed. Existing v0.4.0
installer and executable artifacts are private preview artifacts and remain
outside Git source with their SHA-256 manifest.

Public GitHub publication is intentionally deferred until v0.8.0. This is the
planned product and open-source publication strategy, not a release blocker.

## Publication sequence

- v0.4 through v0.6: local development releases;
- v0.7: public-release hardening and repository-history audit;
- v0.8: first GitHub public pre-release; and
- v1.0: stable public release.

## v0.8 publication gate

Before the first public pre-release, complete and validate:

- a full repository-history secret scan;
- author-name and email review;
- absolute-path review;
- generated-file and binary-history review;
- license review;
- release-tag verification;
- a fresh-clone build;
- public documentation review;
- a final security scan; and
- installer and checksum validation.

Preserve complete Git history unless the v0.7 audit identifies sensitive
material that genuinely requires history rewriting.
