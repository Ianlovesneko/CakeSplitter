# Third-Party Notices

CakeSplitter is distributed under the MIT License in `LICENSE`. This notice is
an engineering attribution summary based on the committed `package-lock.json`,
`Cargo.lock`, local package metadata, and the machine-readable inventory in
[`docs/v0.7-third-party-license-inventory.json`](docs/v0.7-third-party-license-inventory.json).
It is not legal advice. Before v0.8 publication, repeat the review against the
exact public build outputs and preserve the applicable upstream license text.

## Rust runtime and build dependencies

The Rust lockfile contains 466 package records. Workspace crates are MIT. The
normal dependency graph includes permissive MIT, Apache-2.0, BSD, ISC, Zlib,
Unicode-3.0, Unlicense/MIT, and related dual-license expressions.

The following transitive crates require explicit attribution review because
their metadata includes MPL-2.0:

- `cssparser` 0.36.0;
- `cssparser-macros` 0.6.1;
- `dtoa-short` 0.3.5;
- `option-ext` 0.2.0; and
- `selectors` 0.36.1.

The two `r-efi` records (5.3.0 and 6.0.0) declare
`MIT OR Apache-2.0 OR LGPL-2.1-or-later`; the permissive MIT/Apache choices are
available in their metadata, so this is recorded as a dual-license review
item rather than an assertion that LGPL terms are being selected.

`rusqlite` and `libsqlite3-sys` are used with the bundled SQLite feature. Their
upstream MIT license text is present in the local registry; the bundled SQLite
amalgamation carries its upstream public-domain dedication. The exact source
and license notices must be retained with any distributed binary package.

Tauri, WebView, and related runtime crates expose Apache-2.0/MIT or compatible
metadata. Their upstream license files remain the authoritative notices.

## npm, Web, and Desktop dependencies

The committed npm lockfile contains 285 package records. The normalized
inventory records 229 MIT, 19 Apache-2.0, 13 ISC, 13 Apache-2.0 OR MIT, six
BSD-2-Clause, two BSD-3-Clause, and one each of BlueOak-1.0.0, Python-2.0, and
CC-BY-4.0.

The notable non-default metadata items are:

- `minimatch` 10.2.5 — BlueOak-1.0.0, build/development dependency;
- `argparse` 2.0.1 — Python-2.0, development dependency; and
- `caniuse-lite` 1.0.30001806 — CC-BY-4.0 build metadata used by the frontend
  toolchain, not selected-file content or a network runtime.

React, the shared workspace packages, and `@tauri-apps/api` are the relevant
runtime-side JavaScript dependencies. Vite, the React plugin, the Tauri CLI,
test tools, and their transitive toolchains are build/development-only. The
inventory marks platform-optional packages that are present only for another
operating system instead of claiming they are in the Windows output.

## Assets and examples

The repository contains 52 tracked icon assets totaling 272,806 bytes. They
are project-authored vector artwork or generated derivatives from the checked-in
CakeSplitter/SplitTheCake SVG sources. No bundled third-party font, screenshot,
or external image was identified. Example JSON and test fixtures are synthetic
project content and contain no private user data.

## Publication rule

This summarized notice is sufficient for the v0.7.0 local publication
candidate. A public v0.8 release must verify the exact bundled files, preserve
applicable license texts and notices, and resolve any newly detected package or
asset provenance concern before publication.
