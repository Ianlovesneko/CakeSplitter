# GitHub Repository Settings Checklist for v0.8

Verify these settings through authenticated GitHub tooling after `main` exists:

- repository is public and the default branch is `main`;
- GitHub Private Vulnerability Reporting is enabled;
- dependency graph and Dependabot alerts are enabled;
- secret scanning and push protection are enabled where the public-repository
  plan supports them;
- force pushes and branch deletion are disabled for protected `main`;
- future changes use pull requests where appropriate;
- issue templates, `SECURITY.md`, `LICENSE`, and third-party notices render
  publicly; and
- only `main` plus intended release tags are published.

Do not claim a setting is enabled until a read-back from GitHub confirms it.
