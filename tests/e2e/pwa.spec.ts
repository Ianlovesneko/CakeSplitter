import { expect, test, type Download } from '@playwright/test';

test('the complete application shell launches and processes locally offline', async ({
  context,
  page,
}) => {
  const requests: Array<{ method: string; url: string; postData: string | null }> = [];
  page.on('request', (request) => {
    requests.push({ method: request.method(), url: request.url(), postData: request.postData() });
  });

  await page.goto('/');
  await page.evaluate(async () => navigator.serviceWorker.ready);
  await page.reload();
  await expect.poll(() => page.evaluate(() => Boolean(navigator.serviceWorker.controller))).toBe(true);

  const cacheAudit = await page.evaluate(async () => {
    const cacheNames = await caches.keys();
    const entries = [];
    for (const cacheName of cacheNames) {
      const cache = await caches.open(cacheName);
      for (const request of await cache.keys()) {
        const response = await cache.match(request);
        entries.push({ url: request.url, ok: response?.ok === true });
      }
    }
    return { cacheNames, entries };
  });
  expect(cacheAudit.cacheNames).toEqual(['cakesplitter-shell-v0.3.0']);
  expect(cacheAudit.entries.every((entry) => entry.ok)).toBe(true);
  expect(cacheAudit.entries.some((entry) => /\/assets\/index-[\w-]+\.js$/u.test(entry.url))).toBe(true);
  expect(cacheAudit.entries.some((entry) => /\/assets\/worker-[\w-]+\.js$/u.test(entry.url))).toBe(true);
  expect(
    cacheAudit.entries.every((entry) => {
      const url = new URL(entry.url);
      return url.origin === 'http://127.0.0.1:4173' &&
        (['/', '/index.html', '/manifest.webmanifest', '/icon.svg'].includes(url.pathname) ||
          /^\/assets\/[\w.-]+$/u.test(url.pathname));
    }),
  ).toBe(true);

  await context.setOffline(true);
  await page.reload({ waitUntil: 'domcontentloaded' });
  await expect(page.getByText('Offline · local-only')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Split' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Merge' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Inspect' })).toBeVisible();

  const downloads: Download[] = [];
  page.on('download', (download) => downloads.push(download));
  await page.getByTestId('split-file').setInputFiles({
    name: 'offline-local.bin',
    mimeType: 'application/octet-stream',
    buffer: Buffer.from('offline bytes'),
  });
  await page.getByLabel('Target Slice size in bytes').fill('64');
  await page.getByRole('button', { name: 'Cut the Cake' }).click();
  await expect(page.getByText('Cake cut into 1 verified Slice.')).toBeVisible();
  await expect.poll(() => downloads.length).toBe(2);

  const privateTraffic = requests.filter(
    (request) =>
      request.method !== 'GET' ||
      request.postData !== null ||
      request.url.includes('offline-local.bin') ||
      request.url.endsWith('.slice') ||
      request.url.endsWith('.cake.json'),
  );
  expect(privateTraffic).toEqual([]);
  const postProcessingAudit = await page.evaluate(async () => {
    const names = await caches.keys();
    const urls = (
      await Promise.all(names.map(async (name) => (await (await caches.open(name)).keys()).map((entry) => entry.url)))
    ).flat();
    return urls;
  });
  expect(postProcessingAudit.some((url) => /offline-local|\.slice$|\.cake\.json$/u.test(url))).toBe(false);
});
