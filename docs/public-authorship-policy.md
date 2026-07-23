# Public Authorship Policy

Status: decision support for the private v0.7.0-dev development line. This
document does not rewrite history or change local Git configuration.

## Reviewed identity

The reachable history contains 20 commits and five annotated release tags. All
reviewed author and committer records use the intended name, **Yu-En Huang**,
and the same historical iCloud-domain address, summarized here as
`e••••••@icloud.com`. The address first appears in the first reachable commit
and remains the latest historical identity, including the v0.6.0 tag and the
current v0.7 contract-alignment checkpoint. Taggers use the same identity.

No complete private email address is reproduced in public documentation.

## Decision options

### Option A — Preserve existing history unchanged (recommended)

- retain historical author and committer addresses;
- use a verified public address or GitHub noreply address for future commits;
- keep the complete engineering history and existing tag targets; and
- perform no history rewrite before v0.8 unless a later audit validates a
  concrete privacy or safety issue.

### Option B — Rewrite historical author email

Rewrite every reachable commit and affected annotated tag before the first
public push. This would invalidate commit and tag hashes, require new bundles
and full revalidation, and create higher operational risk.

### Option C — Create a new public history root at v0.8

Keep the private history separately and publish a sanitized initial public
commit. This reduces historical identity exposure but loses visible engineering
history on GitHub and requires a separate evidence-preservation process.

## Policy

Option A is recommended because the completed history audit found no validated
secret, real local path, generated-file leak, or other concrete safety issue
requiring history rewriting. Before v0.8 publication, future commits should
use a verified public identity, preferably a GitHub noreply address associated
with the publishing account. Future annotated tags should use that same public
tagger identity and be verified as part of the release gate.

This project does not sign tags in this checkpoint. No remote is configured and
no public publication is authorized by this document.

## User decision still required

Before v0.8, the project owner must explicitly confirm Option A, B, or C and
authorize the exact public commit email (or GitHub noreply address). Until that
decision is recorded and verified, the repository remains a private local
development source tree.
