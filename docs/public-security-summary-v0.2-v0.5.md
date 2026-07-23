# Public Security Summary — v0.2 through v0.5

This summary is the publishable high-level companion to the detailed private
historical reports. It intentionally omits scan identifiers, snapshot names,
backup locations, local paths, private identities, and raw proof procedures.

## v0.2.1

The first repository-wide review retained five Medium and seven Low findings
across browser output handling, native publication races, terminal output, and
verification presentation. The follow-up hardening closed the validated issues
with fail-closed browser behavior, native identity checks, no-replace output
publication, terminal-safe diagnostics, and runtime validation. No Critical or
High finding was validated.

## v0.3.0

The focused review validated two Medium and two Low findings involving source
stability, service-worker shell binding, browser persistence barriers, and
terminal-control output. All four were remediated with production-path checks
and regression coverage. Direct Folder Mode remained disabled because the
browser runtime could not prove portable atomic no-replace publication.

## v0.4.0

The native desktop review validated three Medium and four Low findings covering
destination and selection identity, task admission, package binding, bounded
enumeration, and recovery behavior. The production controls use Windows object
identity, serialized Rust admission, durable checksums, revalidation, and
fail-closed recovery. No unresolved Critical, High, or actionable Medium issue
remained at the local-release checkpoint.

## v0.5.0

The focused desktop review validated four Medium and five Low findings and one
suppressed false positive. The remediations covered export and diagnostic
directory races, diagnostic path redaction, orphaned partial cleanup, queue
and recovery boundaries, and IPC validation. No Critical or High finding was
identified in that focused review.

## Accepted limitations

These historical releases were local previews. SHA-256 provides byte integrity,
not publisher authenticity; Cake Package 1.0 has no signatures or encryption;
Windows filesystem semantics can vary; browser Compatibility Mode buffers data
under explicit limits; and public GitHub publication remains deferred until
v0.8. Detailed historical evidence remains private until a separately
authorized history/publication audit decides otherwise.
