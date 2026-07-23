# Release Signing Policy

`TAG_SIGNING_POLICY = unsigned-until-v0.8`.

The v0.7.0 local publication-candidate tag is an ordinary annotated tag. No
GPG or SSH signing key is generated, imported, or assumed. Historical release
tags remain unsigned and are not recreated solely for signing.

Before the v0.8 public launch, configure an authorized signing key, verify its
identity, sign the public release tag, and publish the verification method with
the release instructions. Unsigned local preview artifacts must remain clearly
identified as such.
