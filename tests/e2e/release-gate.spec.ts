import { createHash, randomUUID } from 'node:crypto';

import { expect, test, type Download } from '@playwright/test';

test('drag-and-drop Slice-count planning produces exact verified downloads', async ({ page }) => {
  const downloads: Download[] = [];
  page.on('download', (download) => downloads.push(download));

  await page.goto('/');
  await page.locator('.drop-zone').evaluate((element) => {
    const transfer = new DataTransfer();
    transfer.items.add(
      new File([new TextEncoder().encode('abcdefghij')], 'drag-count.bin', {
        type: 'application/octet-stream',
      }),
    );
    element.dispatchEvent(
      new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: transfer }),
    );
  });
  await page.getByRole('button', { name: 'Slice count' }).click();
  await page.getByLabel('Requested Slice count').fill('3');
  await expect(page.getByText('drag-count.bin · 10 bytes · 3 planned Slices')).toBeVisible();
  await expect(page.getByText('4 bytes target size')).toBeVisible();

  await page.getByRole('button', { name: 'Cut the Cake' }).click();
  await expect(page.getByText('Cake cut into 3 verified Slices.')).toBeVisible();
  await expect.poll(() => downloads.length).toBe(4);

  const byName = new Map(downloads.map((download) => [download.suggestedFilename(), download]));
  const manifestDownload = byName.get('drag-count.bin.cake.json');
  if (!manifestDownload) throw new Error('Manifest download was not emitted.');
  const manifest = JSON.parse((await downloadBytes(manifestDownload)).toString('utf8')) as {
    original: { filename: string; size: number; sha256: string };
    targetSliceSize: number;
    sliceCount: number;
    slices: Array<{ filename: string; size: number; sha256: string }>;
  };
  const rebuilt: Buffer[] = [];
  for (const entry of manifest.slices) {
    const download = byName.get(entry.filename);
    if (!download) throw new Error(`Missing download ${entry.filename}`);
    const bytes = await downloadBytes(download);
    expect(bytes).toHaveLength(entry.size);
    expect(sha256(bytes)).toBe(entry.sha256);
    rebuilt.push(bytes);
  }
  const combined = Buffer.concat(rebuilt);
  expect(combined).toEqual(Buffer.from('abcdefghij'));
  expect(manifest).toMatchObject({
    original: {
      filename: 'drag-count.bin',
      size: 10,
      sha256: sha256(combined),
    },
    targetSliceSize: 4,
    sliceCount: 3,
  });
});

test('Inspect reports missing Slices, accepts reordered Slices, and invalid manifests never render success', async ({ page }) => {
  const fixture = packageFixture();
  await page.goto('/');
  await page.getByRole('button', { name: 'Inspect' }).click();
  await page.getByTestId('manifest-file').setInputFiles(fixture.manifest);
  await page.getByTestId('slice-files').setInputFiles([fixture.slices[0]!]);
  await page.getByRole('button', { name: 'Inspect Package' }).click();
  await expect(page.getByRole('heading', { name: 'Package needs attention' })).toBeVisible();
  await expect(page.getByText('1 missing')).toBeVisible();
  await expect(page.getByText('Package verified', { exact: true })).toHaveCount(0);

  await page.getByTestId('slice-files').setInputFiles([...fixture.slices].reverse());
  await page.getByRole('button', { name: 'Inspect Package' }).click();
  await expect(page.getByRole('heading', { name: 'Package verified' })).toBeVisible();

  await page.getByRole('button', { name: 'Merge' }).click();
  await page.getByTestId('slice-files').setInputFiles([...fixture.slices].reverse());
  const rebuiltPromise = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Layer the Cake' }).click();
  expect(await downloadBytes(await rebuiltPromise)).toEqual(fixture.original);
  await expect(page.getByText(/final SHA-256 matches the manifest/u)).toBeVisible();

  await page.getByRole('button', { name: 'Inspect' }).click();

  const invalidCases = [
    {
      name: 'unsupported-format.cake.json',
      body: JSON.stringify({ ...fixture.value, format: 'other' }),
      expected: /Unsupported format identifier/u,
    },
    {
      name: 'unsupported-version.cake.json',
      body: JSON.stringify({ ...fixture.value, version: '2.0' }),
      expected: /Unsupported format version/u,
    },
    {
      name: 'unsafe-name.cake.json',
      body: JSON.stringify({
        ...fixture.value,
        original: { ...fixture.value.original, filename: '../unsafe.bin' },
      }),
      expected: /Unsafe or invalid portable filename/u,
    },
    {
      name: 'malformed.cake.json',
      body: '{not valid JSON',
      expected: /Manifest is not valid JSON/u,
    },
  ];
  for (const invalid of invalidCases) {
    await page.getByTestId('manifest-file').setInputFiles({
      name: invalid.name,
      mimeType: 'application/json',
      buffer: Buffer.from(invalid.body),
    });
    await expect(page.getByText(invalid.expected)).toBeVisible();
    await expect(page.getByRole('button', { name: 'Inspect Package' })).toBeDisabled();
    await expect(page.getByText('Package verified', { exact: true })).toHaveCount(0);
  }
});

test('OPFS quota failure is disclosed and a failed Clear All blocks new work', async ({ page }) => {
  await page.addInitScript(() => {
    const storage = navigator.storage as StorageManager & {
      getDirectory(): Promise<FileSystemDirectoryHandle>;
    };
    Object.defineProperty(storage, 'getDirectory', {
      configurable: true,
      value: () =>
        Promise.reject(new DOMException('Simulated OPFS quota failure.', 'QuotaExceededError')),
    });
  });

  await page.goto('/');
  await page.getByRole('button', { name: 'Tasks' }).click();
  await expect(page.getByText('Simulated OPFS quota failure.')).toBeVisible();
  page.once('dialog', async (dialog) => dialog.accept());
  await page.getByRole('button', { name: 'Clear all local data' }).click();
  await expect(page.getByText(/Clear All did not complete: Simulated OPFS quota failure/u)).toBeVisible();
  await expect(page.getByText(/Browser-local cleanup failed closed/u)).toBeVisible();

  await page.getByRole('button', { name: 'Split' }).click();
  await page.getByTestId('split-file').setInputFiles({
    name: 'blocked-after-clear.bin',
    mimeType: 'application/octet-stream',
    buffer: Buffer.from('blocked'),
  });
  await page.getByLabel('Target Slice size in bytes').fill('64');
  await page.getByRole('button', { name: 'Cut the Cake' }).click();
  await expect(
    page.getByText('Browser-local cleanup previously failed closed. Reload before starting another task.'),
  ).toBeVisible();
  await expect(page.getByText('Processing', { exact: true })).toHaveCount(0);
});

test('privacy APIs remain unused and release UI is accessible and truthful', async ({ page }) => {
  const requests: Array<{ method: string; url: string; postData: string | null }> = [];
  page.on('request', (request) => {
    requests.push({ method: request.method(), url: request.url(), postData: request.postData() });
  });
  await page.addInitScript(() => {
    type NetworkCall = { api: string; target: string };
    const target = globalThis as typeof globalThis & { __cakeNetworkCalls: NetworkCall[] };
    target.__cakeNetworkCalls = [];
    const record = (api: string, value: unknown) => {
      target.__cakeNetworkCalls.push({ api, target: String(value) });
    };
    const originalFetch = globalThis.fetch.bind(globalThis);
    globalThis.fetch = (...arguments_) => {
      record('fetch', arguments_[0]);
      return originalFetch(...arguments_);
    };
    globalThis.XMLHttpRequest = new Proxy(globalThis.XMLHttpRequest, {
      construct(constructor, arguments_, newTarget) {
        record('XMLHttpRequest', 'constructed');
        return Reflect.construct(constructor, arguments_, newTarget) as XMLHttpRequest;
      },
    });
    const originalBeacon = navigator.sendBeacon.bind(navigator);
    Object.defineProperty(navigator, 'sendBeacon', {
      configurable: true,
      value: (url: string | URL, data?: BodyInit | null) => {
        record('sendBeacon', url);
        return originalBeacon(url, data);
      },
    });
    globalThis.WebSocket = new Proxy(globalThis.WebSocket, {
      construct(constructor, arguments_, newTarget) {
        record('WebSocket', arguments_[0]);
        return Reflect.construct(constructor, arguments_, newTarget) as WebSocket;
      },
    });
    globalThis.EventSource = new Proxy(globalThis.EventSource, {
      construct(constructor, arguments_, newTarget) {
        record('EventSource', arguments_[0]);
        return Reflect.construct(constructor, arguments_, newTarget) as EventSource;
      },
    });
  });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/');

  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Skip to workspace' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('#workspace')).toBeFocused();
  await expect(page.getByText('Processed locally in your browser. Your files never leave your device.')).toBeVisible();
  await expect(page.getByRole('radio', { name: /Direct Folder Mode/u })).toBeDisabled();
  await expect(page.getByText(/does not expose atomic no-replace finalization/u).first()).toBeVisible();

  const documentAudit = await page.evaluate(() => {
    const ids = [...document.querySelectorAll<HTMLElement>('[id]')].map((element) => element.id);
    const duplicateIds = ids.filter((id, index) => ids.indexOf(id) !== index);
    const unlabeledInputs = [...document.querySelectorAll<HTMLInputElement>('input')]
      .filter((input) => (input.labels?.length ?? 0) === 0 && !input.getAttribute('aria-label'))
      .map((input) => input.id || input.type);
    return {
      duplicateIds,
      unlabeledInputs,
      reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
      scrollBehavior: getComputedStyle(document.documentElement).scrollBehavior,
    };
  });
  expect(documentAudit).toEqual({
    duplicateIds: [],
    unlabeledInputs: [],
    reducedMotion: true,
    scrollBehavior: 'auto',
  });

  const downloads: Download[] = [];
  page.on('download', (download) => downloads.push(download));
  await page.getByTestId('split-file').setInputFiles({
    name: 'privacy-local.bin',
    mimeType: 'application/octet-stream',
    buffer: Buffer.from('privacy bytes'),
  });
  await page.getByLabel('Target Slice size in bytes').fill('64');
  await page.getByRole('button', { name: 'Cut the Cake' }).click();
  await expect(page.getByText('Cake cut into 1 verified Slice.')).toBeVisible();
  await expect.poll(() => downloads.length).toBe(2);

  await page.getByRole('button', { name: 'Tasks' }).click();
  await expect(page.getByRole('region', { name: 'Browser-local task storage' })).toBeVisible();
  page.once('dialog', async (dialog) => dialog.accept());
  await page.getByRole('button', { name: 'Clear all local data' }).click();
  await expect(page.getByText('All browser-local CakeSplitter task metadata was cleared.')).toBeVisible();
  await page.getByRole('button', { name: 'About' }).click();
  await expect(page.getByText(/CakeSplitter Desktop is a separate Windows x64 application\./u)).toBeVisible();
  await expect(page.getByText('Cake Package is a project format, not an industry standard.')).toBeVisible();

  const policy = await page.locator('meta[http-equiv="Content-Security-Policy"]').getAttribute('content');
  expect(policy).toContain("connect-src 'none'");
  const apiCalls = await page.evaluate(
    () =>
      (globalThis as typeof globalThis & {
        __cakeNetworkCalls: Array<{ api: string; target: string }>;
      }).__cakeNetworkCalls,
  );
  expect(apiCalls).toEqual([]);
  expect(
    requests.filter((request) => {
      const url = new URL(request.url);
      return request.method !== 'GET' || request.postData !== null || url.origin !== 'http://127.0.0.1:4173';
    }),
  ).toEqual([]);
});

function packageFixture() {
  const filename = 'release-package.bin';
  const original = Buffer.from('release gate package bytes');
  const targetSliceSize = 13;
  const chunks = [original.subarray(0, targetSliceSize), original.subarray(targetSliceSize)];
  const slices = chunks.map((chunk, position) => ({
    name: `${filename}.${String(position + 1).padStart(3, '0')}.slice`,
    mimeType: 'application/octet-stream',
    buffer: Buffer.from(chunk),
  }));
  const value = {
    format: 'cakesplitter',
    version: '1.0',
    packageId: randomUUID(),
    createdAt: '2026-07-17T12:00:00.000Z',
    original: { filename, size: original.length, sha256: sha256(original) },
    targetSliceSize,
    sliceCount: slices.length,
    slices: chunks.map((chunk, position) => ({
      index: position + 1,
      filename: slices[position]!.name,
      offset: position * targetSliceSize,
      size: chunk.length,
      sha256: sha256(chunk),
    })),
  };
  return {
    original,
    value,
    manifest: {
      name: `${filename}.cake.json`,
      mimeType: 'application/json',
      buffer: Buffer.from(JSON.stringify(value)),
    },
    slices,
  };
}

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}

async function downloadBytes(download: Download): Promise<Buffer> {
  const stream = await download.createReadStream();
  const chunks: Buffer[] = [];
  for await (const chunk of stream) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk as Uint8Array));
  }
  return Buffer.concat(chunks);
}
