# Desktop Support

CakeSplitter Desktop v0.6.0 is a completed local Windows x64 prototype release.
It is not publicly distributed; publication is planned for v0.8.0.

## Supported release target

| Item | v0.6.0 support |
| --- | --- |
| Operating system | Windows 10 and Windows 11 |
| Architecture | x64 |
| Installer | NSIS, per-user/current-user |
| Web runtime | Microsoft Edge WebView2 |
| Filesystem | Local Windows filesystems with stable native identity |
| Application version | 0.6.0 |
| Cake Package format | 1.0 |
| Signing | Unsigned preview |

The installer does not request elevation in current-user mode. Because it is
unsigned, Windows SmartScreen may warn that the publisher is unknown. Verify
the release SHA-256 before running it.

## Filesystem boundary

The supported security model depends on Windows volume/file identity,
non-delete-sharing directory handles, reparse-point rejection, and atomic
no-replace publication. Network shares, removable media, cloud-synchronized
folders, virtualized paths, identity-poor filesystems, and unusual filter
drivers may not provide equivalent semantics. CakeSplitter fails closed when
it cannot prove stable identity; users may need to reselect a local NTFS path.

Cross-volume Split and Merge inputs are allowed when each selected object and
output filesystem independently satisfy the checks. Final publication itself
occurs within the selected output directory; CakeSplitter does not use a
cross-volume rename as an atomic publication primitive.

## Not supported by this release

- macOS, Linux, or ARM64 desktop packages;
- signed or Trusted Publisher installers;
- background services or automatic updates;
- arbitrary-byte restart resume;
- package signatures, encryption, compression, or cloud transfer; and
- a plugin runtime or marketplace.

SplitTheCake Web remains available as a separate browser runtime with lower,
explicit Compatibility Mode limits. Web Direct Folder Mode is disabled.
