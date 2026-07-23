# Native Filesystem Security

CakeSplitter Desktop `v0.7.0` treats every path and filesystem object as mutable
and potentially adversarial. Path text, canonicalization, existence, size, and
timestamps alone are not authorization.

## Selection binding

Native pickers create short-lived opaque tokens. File tokens bind Windows volume
and file identity, length, and modification time; directory tokens bind volume
and directory identity. Package selections additionally bind the manifest hash,
package ID, directory identity, ordered Slice membership, and every Slice
fingerprint. Future output tokens bind the selected parent and require the final
name to remain absent.

Every token resolution reopens the selected object and compares the binding.
Replacement, rebinding, hard-link aliasing, permission loss, reparse ambiguity,
or an inaccessible identity returns a structured error and requires reselection.
The stored identity is never silently updated to a replacement object.

## Destination authority

During native Split and Merge, CakeSplitter retains the selected output
directory and replaceable ancestor handles. On Windows those handles omit delete
sharing, which blocks directory rename, deletion, junctioning, or replacement
while publication is active. The runtime revalidates retained and path-resolved
identity before creation, at checkpoints, before and after each Slice publish,
before the final manifest or rebuilt file, and before returning success.

Existing reparse points and reparse points introduced at a checked boundary are
rejected. A failed identity proof stops further publication. No final manifest
or rebuilt output is announced as complete.

## Atomic publication

Task-owned outputs begin as exclusive `.partial` files. CakeSplitter records
their identity, size, and expected SHA-256, flushes and synchronizes them,
reopens and revalidates them, then invokes a native no-replace publication
operation. A destination created after preflight is preserved and produces a
structured collision error.

Slices publish before the manifest. The manifest is the package completion
marker. Cleanup deletes only an identity-owned partial; ambiguous paths are left
for manual local cleanup rather than followed by name.

## Accepted platform limits

The release claim is Windows 10/11 x64 on local filesystems with stable native
identity. Network, removable, synchronized, virtualized, or identity-poor
filesystems may fail closed. A package is not claimed immutable after an
operation returns; later changes are detected by durable binding, Inspect,
Verify, or Merge. SHA-256 provides integrity, not publisher authenticity.
