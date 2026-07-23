import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

import { describe, expect, it } from 'vitest';

const SOURCE = readFileSync(
  fileURLToPath(new URL('../public/sw.js', import.meta.url)),
  'utf8',
);
const ORIGIN = 'https://cakesplitter.example';
const CURRENT_CACHE = 'cakesplitter-shell-v0.7.0-dev';
const SHELL_MARKER = '<meta name="application-name" content="CakeSplitter"';
const SHELL_HTML = `<!doctype html><html><head>${SHELL_MARKER} /></head><body><div id="root"></div></body></html>`;

function response(body, options = {}) {
  const value = new Response(body, {
    status: options.status ?? 200,
    headers: { 'content-type': options.contentType ?? 'text/html; charset=utf-8' },
  });
  Object.defineProperties(value, {
    redirected: { value: options.redirected ?? false },
    type: { value: options.type ?? 'basic' },
    url: { value: options.url ?? `${ORIGIN}/index.html` },
  });
  return value;
}

function createHarness({ fetchImpl, cacheKeys = [CURRENT_CACHE], offlineShell } = {}) {
  const listeners = new Map();
  const puts = [];
  const matches = [];
  const deleted = [];
  let claimed = false;

  const cache = {
    async addAll() {},
    async match(key) {
      matches.push(String(key));
      return offlineShell;
    },
    async put(key, value) {
      puts.push({ key: String(key), body: await value.text() });
    },
  };
  const context = {
    Error,
    Promise,
    Response,
    Set,
    URL,
    caches: {
      async delete(key) {
        deleted.push(key);
        return true;
      },
      async keys() {
        return cacheKeys;
      },
      async match(key) {
        matches.push(String(key));
        return offlineShell;
      },
      async open() {
        return cache;
      },
    },
    fetch:
      fetchImpl ??
      (async () => {
        throw new Error('offline');
      }),
    self: {
      clients: {
        async claim() {
          claimed = true;
        },
      },
      location: { origin: ORIGIN },
      async skipWaiting() {},
      addEventListener(type, listener) {
        listeners.set(type, listener);
      },
    },
  };
  vm.runInNewContext(SOURCE, context, { filename: 'apps/web/public/sw.js' });
  return {
    deleted,
    listeners,
    matches,
    puts,
    wasClaimed: () => claimed,
  };
}

async function dispatchFetch(harness, pathname, destination = 'document') {
  const waits = [];
  let responsePromise;
  harness.listeners.get('fetch')({
    request: {
      destination,
      method: 'GET',
      mode: 'navigate',
      url: `${ORIGIN}${pathname}`,
    },
    respondWith(value) {
      responsePromise = Promise.resolve(value);
    },
    waitUntil(value) {
      waits.push(Promise.resolve(value));
    },
  });
  const result = await responsePromise;
  await Promise.all(waits);
  return result;
}

async function dispatchActivate(harness) {
  const waits = [];
  harness.listeners.get('activate')({
    waitUntil(value) {
      waits.push(Promise.resolve(value));
    },
  });
  await Promise.all(waits);
}

describe('service-worker application-shell cache policy', () => {
  it('refreshes the canonical shell only with the marked CakeSplitter HTML response', async () => {
    const harness = createHarness({
      fetchImpl: async () => response(SHELL_HTML, { url: `${ORIGIN}/index.html` }),
    });

    await dispatchFetch(harness, '/index.html');

    expect(harness.puts).toEqual([{ key: '/index.html', body: SHELL_HTML }]);
  });

  it('does not let an alternate successful navigation replace the shell', async () => {
    const harness = createHarness({
      fetchImpl: async () =>
        response('{"name":"CakeSplitter"}', {
          contentType: 'application/manifest+json',
          url: `${ORIGIN}/manifest.webmanifest`,
        }),
    });

    await dispatchFetch(harness, '/manifest.webmanifest');

    expect(harness.puts).toEqual([]);
  });

  it('does not let a redirect response replace the shell', async () => {
    const harness = createHarness({
      fetchImpl: async () =>
        response(SHELL_HTML, {
          redirected: true,
          url: `${ORIGIN}/alternate-shell`,
        }),
    });

    await dispatchFetch(harness, '/');

    expect(harness.puts).toEqual([]);
  });

  it('does not let unexpected unmarked HTML replace the shell', async () => {
    const harness = createHarness({
      fetchImpl: async () =>
        response('<!doctype html><title>Unexpected</title>', { url: `${ORIGIN}/` }),
    });

    await dispatchFetch(harness, '/');

    expect(harness.puts).toEqual([]);
  });

  it('does not cache an opaque response as the application shell', async () => {
    const harness = createHarness({
      fetchImpl: async () =>
        response(SHELL_HTML, {
          type: 'opaque',
          url: `${ORIGIN}/index.html`,
        }),
    });

    await dispatchFetch(harness, '/index.html');

    expect(harness.puts).toEqual([]);
  });

  it('uses only the canonical trusted shell key for offline navigation', async () => {
    const offlineShell = response(SHELL_HTML);
    const harness = createHarness({ offlineShell });

    const result = await dispatchFetch(harness, '/');

    expect(harness.matches).toEqual(['/index.html']);
    expect(await result.text()).toBe(SHELL_HTML);
  });

  it('removes only obsolete CakeSplitter application caches during activation', async () => {
    const harness = createHarness({
      cacheKeys: [CURRENT_CACHE, 'cakesplitter-shell-v0.2.1', 'unrelated-app-cache'],
    });

    await dispatchActivate(harness);

    expect(harness.deleted).toEqual(['cakesplitter-shell-v0.2.1']);
    expect(harness.wasClaimed()).toBe(true);
  });
});
