# Public Security Report Index

This index classifies tracked security, privacy, threat, remediation, and
repository-audit documents for a future v0.8 public release. The current
repository remains private. Detailed historical reports are not deleted or
rewritten in this checkpoint; reports marked private must not be copied into a
public release root without a separately authorized history audit.

| Report | Version | Public status | Sanitized/public path | Private raw evidence | Superseding or related report |
|---|---|---|---|---|---|
| `docs/v0.2-security-report.md` | v0.2.1 | retain private | `docs/public-security-summary-v0.2-v0.5.md` | yes; contains internal scan identifiers and detailed findings | summary above |
| `docs/v0.3-security-report.md` | v0.3.0 | publish summary only | `docs/public-security-summary-v0.2-v0.5.md` | yes; contains internal scan metadata | summary above |
| `docs/v0.3-phase11-remediation-report.md` | v0.3.0 | retain private | none | yes; detailed checkpoint and remediation evidence | v0.3 summary |
| `docs/v0.4-security-report.md` | v0.4.0 | publish summary only | `docs/public-security-summary-v0.2-v0.5.md` | yes; canonical internal finding identifiers | v0.4 local-release evidence |
| `docs/v0.4-medium-remediation-report.md` | v0.4.0 | retain private | none | yes; sealed checkpoint and proof details | v0.4 summary |
| `docs/v0.4-low-remediation-report.md` | v0.4.0 | retain private | none | yes; sealed checkpoint and proof details | v0.4 summary |
| `docs/v0.4-native-security-limits.md` | v0.4.0 | publish unchanged | `docs/v0.4-native-security-limits.md` | no private evidence identified | v0.4 security report |
| `docs/v0.5-security-report.md` | v0.5.0 | publish summary only | `docs/public-security-summary-v0.2-v0.5.md` | yes; internal candidate names and review detail | summary above |
| `docs/v0.6-security-review.md` | v0.6.0 | publish unchanged | `docs/v0.6-security-review.md` | no private identity or path identified | v0.6.0 report |
| `docs/v0.6.0-security-report.md` | v0.6.0 | publish unchanged | `docs/v0.6.0-security-report.md` | no private identity or path identified | v0.6 review |
| `docs/v0.6.0-release-audit.md` | v0.6.0 | publish after sanitization | future public release-audit copy | private artifact names and build evidence remain outside public history | v0.7 history audit |
| `docs/v0.7-repository-history-audit.md` | v0.7 | publish after sanitization | this index and future public audit copy | external backup and history-evidence references remain private | v0.7 public metadata checkpoint |
| `docs/v0.7-contract-alignment.md` | v0.7.0 | publish unchanged | `docs/v0.7-contract-alignment.md` | no private identity or path identified | current contract checkpoint |
| `docs/v0.7-final-history-audit.md` | v0.7.0 | publish after ordinary review | `docs/v0.7-final-history-audit.md` | private bundle and hash mapping remain outside Git | supersedes baseline history audit |
| `docs/v0.7.0-security-report.md` | v0.7.0 | publish summary after v0.8 review | `docs/v0.7.0-security-report.md` | raw scan evidence remains outside Git | current local-candidate security result |
| `docs/v0.7-security-publication-classification.md` | v0.7.0 | publish unchanged | `docs/v0.7-security-publication-classification.md` | sealed evidence remains private | current classification |
| `docs/direct-folder-security.md` | current design | publish unchanged | `docs/direct-folder-security.md` | no private evidence identified | browser support docs |
| `docs/native-filesystem-security.md` | current design | publish unchanged | `docs/native-filesystem-security.md` | no private evidence identified | desktop support docs |
| `docs/privacy-model.md` | current design | publish unchanged | `docs/privacy-model.md` | local paths are generic environment placeholders | SECURITY.md |
| `SECURITY.md` | current policy | publish unchanged | `SECURITY.md` | no private contact address | public pre-publication policy |

## Classification rules

Reports were checked for local paths, scan workspaces, private email addresses,
internal candidate IDs, raw proof procedures, machine-specific evidence, stale
statuses, and contradictory publication claims. The sanitized summary keeps
severity, affected security boundary, remediation class, final status, and
accepted limitations while removing unnecessary operational detail.

No raw scan workspace, sealed checkpoint, backup, browser profile, PoC binary,
or private DOCX is copied into the public documentation set. Before v0.8, the
history audit must decide whether the existing private historical files can be
published, summarized, or replaced in a new public history root.
