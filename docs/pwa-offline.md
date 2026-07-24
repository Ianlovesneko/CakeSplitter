# PWA and Offline Operation

CakeSplitter `v0.8.0` has an installable Web App manifest and a service worker
for the static application shell. Offline support does not change the local-only
file-processing model.

## Cached content

The current cache name is `cakesplitter-shell-v0.8.0`. Installation validates
the marked canonical `/index.html`, discovers its hashed assets, and caches only:

- `/`, `/index.html`, `/manifest.webmanifest`, and `/icon.svg`; and
- same-origin flat `/assets/` files referenced by the built shell.

Selected filenames, file contents, Cake Manifests, Slices, rebuilt output,
hashes, package IDs, task metadata, OPFS records, and private errors are not
service-worker cache entries.

## Trusted shell policy

A response may replace the canonical shell only when it is:

- same-origin;
- for `/` or `/index.html` without a query;
- successful HTML;
- neither redirected nor opaque; and
- marked as the CakeSplitter application.

Alternate navigation, a redirect, unmarked HTML, JSON returned with status 200,
an opaque response, and query variants cannot replace the shell. Offline
navigation reads only the canonical `/index.html` key.

## Updates and cleanup

Activation deletes only obsolete caches with the CakeSplitter cache prefix;
unrelated origin caches are untouched. A waiting service worker accepts only
the exact `{ type: "ACTIVATE_UPDATE" }` message. The UI disables update
activation while a Worker task is running or paused, so an update cannot
silently replace code during active processing.

## Offline behavior

After one successful online load and service-worker control, the app can reload
offline and run Compatibility Split, Merge, and Inspect with user-selected
local files. Browser downloads, OPFS, memory, quota, and platform limits still
apply. Offline status is visible in the header.

## Security evidence

Seven service-worker VM tests cover canonical refresh, alternate navigation,
redirects, unmarked HTML, opaque responses, canonical offline fallback, and
ownership-scoped cache cleanup. Two PWA controller tests cover controlled
activation. Production Edge starts offline from the cached shell, performs a
local Split, and confirms no user data appears in network requests or Cache
Storage.

The same-origin static host remains a trust boundary. A compromised host can
serve altered application code before local controls run.
