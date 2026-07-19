/* CakeSplitter application-shell service worker. User file data never uses fetch(). */
const CACHE_PREFIX = 'cakesplitter-shell-';
const CACHE_NAME = `${CACHE_PREFIX}v0.4.0`;
const SHELL_CACHE_KEY = '/index.html';
const SHELL_MARKER = '<meta name="application-name" content="CakeSplitter"';
const SHELL_PATHS = ['/', '/index.html', '/manifest.webmanifest', '/icon.svg'];
const CANONICAL_NAVIGATION_PATHS = new Set(['/', '/index.html']);
const STATIC_SHELL_PATHS = new Set(SHELL_PATHS);
const CACHEABLE_STATIC_DESTINATIONS = new Set([
  'script',
  'style',
  'worker',
  'manifest',
  'image',
  'font',
]);

self.addEventListener('install', (event) => {
  event.waitUntil(cacheApplicationShell());
});

async function cacheApplicationShell() {
  const cache = await caches.open(CACHE_NAME);
  const shellUrl = new URL(SHELL_CACHE_KEY, self.location.origin);
  const response = await fetch(SHELL_CACHE_KEY, { cache: 'reload' });
  if (!(await isTrustedShellResponse(response, shellUrl))) {
    throw new Error('Application shell index could not be validated for caching.');
  }
  const html = await response.clone().text();
  const assetPaths = [...html.matchAll(/(?:src|href)="(\/assets\/[^"]+)"/gu)].map(
    (match) => match[1],
  );
  const paths = [...new Set([...SHELL_PATHS, ...assetPaths])];
  await cache.put(SHELL_CACHE_KEY, response);
  await cache.addAll(paths.filter((path) => path !== SHELL_CACHE_KEY));
  const nestedAssets = new Set();
  for (const path of assetPaths.filter((candidate) => candidate.endsWith('.js'))) {
    const script = await cache.match(path);
    if (!script) continue;
    const source = await script.text();
    for (const match of source.matchAll(/["'](\/assets\/[a-zA-Z0-9._-]+)["']/gu)) {
      if (match[1]) nestedAssets.add(match[1]);
    }
  }
  await cache.addAll([...nestedAssets].filter((path) => !paths.includes(path)));
}

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys
            .filter((key) => key.startsWith(CACHE_PREFIX) && key !== CACHE_NAME)
            .map((key) => caches.delete(key)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('message', (event) => {
  const message = event.data;
  if (
    message &&
    typeof message === 'object' &&
    !Array.isArray(message) &&
    Object.keys(message).length === 1 &&
    message.type === 'ACTIVATE_UPDATE'
  ) {
    event.waitUntil(self.skipWaiting());
  }
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  const url = new URL(request.url);
  if (
    request.method !== 'GET' ||
    url.origin !== self.location.origin ||
    (request.mode !== 'navigate' && !isCacheableStaticRequest(request, url))
  ) {
    return;
  }
  if (request.mode === 'navigate') {
    event.respondWith(
      fetch(request)
        .then(async (response) => {
          if (
            isCanonicalNavigationUrl(url) &&
            (await isTrustedShellResponse(response, url))
          ) {
            const copy = response.clone();
            event.waitUntil(
              caches.open(CACHE_NAME).then((cache) => cache.put(SHELL_CACHE_KEY, copy)),
            );
          }
          return response;
        })
        .catch(() =>
          caches
            .open(CACHE_NAME)
            .then((cache) => cache.match(SHELL_CACHE_KEY))
            .then((response) => response || Response.error()),
        ),
    );
    return;
  }
  event.respondWith(
    caches.open(CACHE_NAME).then((cache) =>
      cache.match(request).then(
        (cached) =>
          cached ||
          fetch(request).then((response) => {
            if (isTrustedStaticResponse(response, url)) {
              const copy = response.clone();
              event.waitUntil(cache.put(request, copy));
            }
            return response;
          }),
      ),
    ),
  );
});

function isCanonicalNavigationUrl(url) {
  return (
    url.origin === self.location.origin &&
    url.search === '' &&
    CANONICAL_NAVIGATION_PATHS.has(url.pathname)
  );
}

async function isTrustedShellResponse(response, requestUrl) {
  if (
    !response.ok ||
    response.redirected ||
    response.type === 'opaque' ||
    response.type === 'opaqueredirect'
  ) {
    return false;
  }
  const finalUrl = new URL(response.url || requestUrl.href, self.location.origin);
  if (!isCanonicalNavigationUrl(finalUrl)) return false;
  const contentType = response.headers.get('content-type') || '';
  if (!/^text\/html(?:;|$)/iu.test(contentType)) return false;
  const html = await response.clone().text();
  return html.includes(SHELL_MARKER);
}

function isCacheableStaticRequest(request, url) {
  if (url.search !== '' || !CACHEABLE_STATIC_DESTINATIONS.has(request.destination)) {
    return false;
  }
  return STATIC_SHELL_PATHS.has(url.pathname) || /^\/assets\/[a-zA-Z0-9._-]+$/u.test(url.pathname);
}

function isTrustedStaticResponse(response, requestUrl) {
  if (
    !response.ok ||
    response.redirected ||
    response.type === 'opaque' ||
    response.type === 'opaqueredirect'
  ) {
    return false;
  }
  const finalUrl = new URL(response.url || requestUrl.href, self.location.origin);
  return (
    finalUrl.origin === self.location.origin &&
    finalUrl.pathname === requestUrl.pathname &&
    finalUrl.search === ''
  );
}
