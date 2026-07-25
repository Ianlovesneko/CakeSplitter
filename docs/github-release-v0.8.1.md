# CakeSplitter v0.8.1

CakeSplitter v0.8.1 is a narrowly scoped development-toolchain security
refresh. It updates the dependency paths affected by
[`GHSA-mh99-v99m-4gvg`](https://github.com/advisories/GHSA-mh99-v99m-4gvg)
without intended product-function, file-format, CLI-contract, privacy,
networking, Desktop, Web Worker, or browser-filesystem changes.

## Security maintenance

The v0.8.0 development dependency tree contained two affected
`brace-expansion` paths:

- `eslint@9.39.5` → `minimatch@3.1.5` →
  `brace-expansion@1.1.16`; and
- `typescript-eslint@8.64.0` →
  `@typescript-eslint/typescript-estree@8.64.0` →
  `minimatch@10.2.5` → `brace-expansion@5.0.7`.

Both paths were development-only. `npm audit --omit=dev` reported zero
shipped-runtime findings, the production dependency tree contained no
`brace-expansion`, and no CakeSplitter product source or existing Web/Desktop
build referenced the package. There was no evidence that the affected path was
included in shipped runtime dependencies.

v0.8.1 updates ESLint to 10.8.0, declares the directly imported
`@eslint/js@10.0.1`, removes the obsolete ESLint 9 subtree, and resolves the
remaining development path to patched `brace-expansion@5.0.8`. No package
override, manual lockfile integrity edit, or forced audit fix was used.

After remediation:

- `npm audit` reports zero vulnerabilities;
- `npm audit --omit=dev` reports zero shipped-runtime findings;
- the resolved graph contains only `brace-expansion@5.0.8`; and
- the advisory is absent from the installed dependency graph.

## Validation

- 150 Rust tests passed; one explicit large-file profile remained ignored in
  the ordinary suite and passed separately against a real 1 GiB source.
- 104 Node tests passed.
- 12 Microsoft Edge production, privacy, offline, and accessibility tests
  passed.
- strict Clippy, formatting, lint, type checking, Rust release builds, Web and
  Desktop production builds, Rust/Web interoperability, packaged CLI and
  Desktop smoke checks, npm audit, and RustSec audit passed.
- Cake Package format 1.0, CLI schema 1, and Batch Job schema 1 remain
  unchanged.

See the
[v0.8.1 test report](https://github.com/Ianlovesneko/CakeSplitter/blob/v0.8.1/docs/v0.8.1-test-report.md)
and
[v0.8.1 security report](https://github.com/Ianlovesneko/CakeSplitter/blob/v0.8.1/docs/v0.8.1-security-report.md)
for the detailed evidence and limitations.

## What CakeSplitter does

CakeSplitter splits large local files into numbered `.slice` files plus a
portable `.cake.json` Manifest, and rebuilds verified Packages across the Rust,
Web, CLI, Batch, and Windows Desktop surfaces.

Processing is local-only. CakeSplitter does not upload selected content and
does not include accounts, telemetry, analytics, cloud fallback, an updater,
remote checksums, or a background service.

## Browser and platform limitations

- Web Direct Folder Mode remains disabled as a fail-closed security decision.
- Browser Compatibility Mode can buffer Slice or rebuilt-output data in
  memory. Very large operations remain constrained by memory, browser download
  behavior, and platform limits.
- Windows x64 executables and the installer are not code-signed and may
  trigger Windows SmartScreen.
- Cake Package 1.0 provides SHA-256 integrity, not publisher authenticity.
- The signed Git tag authenticates source, not Windows executables.

## Run from source

The supported Node.js versions are 20.19+, 22.13+, or 24+.

```powershell
npm ci
npm run dev
```

For the Desktop development build:

```powershell
npm run dev:desktop
```

## Verify downloads

Download `SHA256SUMS.txt`, `SHA256SUMS.txt.sig`, and
`CakeSplitter-v0.8.1-Release-Signing-Key.pub` with the artifacts. Follow
`CHECKSUM-VERIFICATION-v0.8.1.md` to verify the checksum signature and then
verify each artifact's SHA-256 value.

This remains an early public pre-release. No v0.9 functionality is included.
