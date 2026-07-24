# Release Signing Policy

`TAG_SIGNING_POLICY = signed-v0.8-tag-required`.

Historical tags through `v0.7.0` remain the original annotated local-release
tags and are not recreated solely to add signatures. The `v0.8.0` public
pre-release tag is signed with the authorized SSH signing identity and must
pass `git verify-tag v0.8.0` before publication.

A signed Git tag authenticates the selected Git object. It does not mean the
Windows executable or installer is code-signed. Unless a separate Windows
code-signing state is documented and verified, public Windows artifacts remain
unsigned and may trigger Windows SmartScreen.

Consumers should verify the signed tag and compare downloaded artifact hashes
with `SHA256SUMS.txt`. Cake Package format 1.0 itself has no publisher
signature or authenticity layer.
