# Security Contact Policy

## Selected policy

`SECURITY_CONTACT_POLICY = github-private-reporting-at-v0.8`.
`PUBLIC_SECURITY_EMAIL = NOT_APPLICABLE`; no email address is invented.

## Current pre-publication state

CakeSplitter is a private local publication candidate. No public repository,
GitHub Security Advisory channel, or security mailbox is enabled. Keep reports
in the maintainer's private project channel until the public launch gate is
complete.

## v0.8 enablement requirement

Before the first public v0.8 pre-release, enable GitHub private vulnerability
reporting, verify that a report can be submitted and acknowledged, and link the
working channel from `SECURITY.md`. Public issue reports must never contain
unpatched vulnerability details.

If platform private reporting cannot be enabled, publication is blocked until an
exact, user-authorized security email is selected, verified, and documented.

## What to include

Provide the affected version or commit, operating system and browser/runtime,
minimal reproduction steps, realistic impact, relevant logs with secrets
removed, and a synthetic package or fixture when one is needed. Do not attach
real private files, credentials, tokens, or sensitive filenames.

## Privacy and response

Keep the report and follow-up private. Maintainers will acknowledge receipt,
validate the source-to-sink path, assign severity only after validation, and
coordinate a fix and disclosure. No fixed response time is promised.
