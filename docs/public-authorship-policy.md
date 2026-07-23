# Public Authorship Policy

Status: authorized v0.7.0 local-publication-candidate policy. The selected
policy is Option B, with the exact public identity recorded below.

## Reviewed identity

The reachable history contains 22 commits and five annotated release tags. All
reviewed author, committer, and annotated-tagger records already use the
authorized public identity **Yu-En Huang <enmity.coyotes.3o@icloud.com>**.
Consequently, the Option B transformation is idempotent: every old commit hash,
tag object, tag target, message, timestamp, and tree remains unchanged. The
private old-to-new mapping records this no-op result outside the repository.

## Decision options

### Option A — Preserve existing history unchanged

- retain historical author and committer addresses;
- use a verified public address or GitHub noreply address for future commits;
- keep the complete engineering history and existing tag targets; and
- perform no history rewrite before v0.8 unless a later audit validates a
  concrete privacy or safety issue.

### Option B — Rewrite historical author email (selected)

Rewrite every reachable commit and affected annotated tag before the first
public push. This would invalidate commit and tag hashes, require new bundles
and full revalidation, and create higher operational risk.

### Option C — Create a new public history root at v0.8

Keep the private history separately and publish a sanitized initial public
commit. This reduces historical identity exposure but loses visible engineering
history on GitHub and requires a separate evidence-preservation process.

## Authorized policy

- `AUTHORSHIP_POLICY = B`.
- `PUBLIC_AUTHOR_NAME = Yu-En Huang`.
- `PUBLIC_COMMIT_EMAIL = enmity.coyotes.3o@icloud.com`.
- Future commits and annotated tags use the same repository-local identity.
- The verified Option B transform required no object rewrite because the
  existing reachable history already matched the authorized identity.
- The original and post-policy bundles, plus the old-to-new mapping, are kept
  outside the repository as private preservation evidence.
- No remote is configured and no public publication is authorized before v0.8.
