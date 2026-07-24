# CakeSplitter v0.7

CakeSplitter is a local-first tool for splitting large files into verified
Slices and rebuilding them exactly. SplitTheCake is the product website and
browser experience for the same Cake Package model.

This repository contains the public v0.7 development source preview. Packaged
downloads are not published yet; the first packaged release remains planned
for v0.8.

## What is included

- A browser workflow that splits files locally in a Web Worker.
- Per-Slice and original-file SHA-256 verification.
- Downloadable `.slice` parts and a `.cake.json` manifest.
- A rebuild workflow that verifies every Slice before recreating the file.
- The SplitTheCake product site, About, Privacy, and Contact pages.
- Cake Package format 1.0 examples and product documentation.

## Current status

| Surface | v0.7 status |
| --- | --- |
| Web App | Available in this repository and linked from the product site |
| GitHub source | Public development preview |
| Windows Desktop | Product roadmap / preview capability |
| CLI and automation | Product roadmap / development capability |
| Packaged release | Planned for v0.8 |

The 21-page PDF under `public/docs/` documents v0.5.0 and is retained as an
archived product reference. The current source of truth for implementation is
this v0.7 repository.

## Run locally

Requires Node.js 22.13.0 or newer.

```bash
npm ci
npm run dev
```

Then open the local URL printed by the development server.

## Validate

```bash
npm run build
npm run lint
npm run typecheck
npm test
```

## Privacy model

Selected file contents are processed inside the browser. The Web App does not
upload the chosen file to a CakeSplitter service. Ordinary website assets are
still loaded over the network when visiting the site.

## Links

- Product site: https://splitthecake.ianlovesneko.chatgpt.site/
- Source: https://github.com/Ianlovesneko/CakeSplitter
- Contact: enmity.coyotes.3o@icloud.com

## Author and licensing

CakeSplitter is designed and developed by Yu-En Huang.

No license file is currently included. The source is public for review, but no
reuse rights are granted until a license is added.
