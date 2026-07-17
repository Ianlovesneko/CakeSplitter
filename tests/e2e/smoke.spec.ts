import { createHash, randomUUID } from 'node:crypto';
import { mkdir, open } from 'node:fs/promises';
import path from 'node:path';

import { expect, test, type Download, type TestInfo } from '@playwright/test';

test('Split, Inspect, Verify, and Merge stay local and preserve bytes', async ({ page }) => {
  const requests: { method: string; url: string; postData: string | null }[] = [];
  page.on('request', (request) => {
    requests.push({
      method: request.method(),
      url: request.url(),
      postData: request.postData(),
    });
  });

  await page.goto('/');
  await expect(page.getByRole('heading', { name: /Cut a Cake into verified Slices/i })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Compatibility Download Mode active' })).toBeVisible();
  await expect(page.getByRole('radio', { name: /Direct Folder Mode/u })).toBeDisabled();
  await expect(page.getByText(/does not expose atomic no-replace finalization/u).first()).toBeVisible();

  const splitDownloads: Download[] = [];
  page.on('download', (download) => splitDownloads.push(download));
  await page.getByTestId('split-file').setInputFiles({
    name: 'browser-smoke.bin',
    mimeType: 'application/octet-stream',
    buffer: Buffer.from('abcdefghij'),
  });
  await page.getByLabel('Target Slice size in bytes').fill('4');
  await expect(page.getByText('browser-smoke.bin · 10 bytes · 3 planned Slices')).toBeVisible();
  await page.getByRole('button', { name: 'Cut the Cake' }).click();
  await expect(page.getByText('Cake cut into 3 verified Slices.')).toBeVisible();
  await expect.poll(() => splitDownloads.length).toBe(4);
  expect(splitDownloads.map((download) => download.suggestedFilename()).sort()).toEqual([
    'browser-smoke.bin.001.slice',
    'browser-smoke.bin.002.slice',
    'browser-smoke.bin.003.slice',
    'browser-smoke.bin.cake.json',
  ]);

  const fixture = createPackageFixture();
  await page.getByRole('button', { name: 'Inspect' }).click();
  await page.getByTestId('manifest-file').setInputFiles(fixture.manifest);
  await page.getByTestId('slice-files').setInputFiles(fixture.slices);
  await page.getByRole('button', { name: 'Inspect Package' }).click();
  await expect(page.getByRole('heading', { name: 'Package verified' })).toBeVisible();
  await expect(page.getByText('Every expected Slice is present and verified.')).toBeVisible();
  await expect(page.getByText('2 found / 2 expected')).toBeVisible();

  await page.getByRole('button', { name: 'Merge' }).click();
  await page.getByTestId('manifest-file').setInputFiles(fixture.manifest);
  await page.getByTestId('slice-files').setInputFiles(fixture.slices);
  const downloadPromise = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Layer the Cake' }).click();
  const rebuilt = await downloadPromise;
  await expect(page.getByText('Cake rebuilt exactly. The final SHA-256 matches the manifest.')).toBeVisible();
  expect(rebuilt.suggestedFilename()).toBe(fixture.filename);
  expect(await downloadBytes(rebuilt)).toEqual(fixture.original);

  const userDataRequests = requests.filter(
    (request) =>
      request.method !== 'GET' ||
      request.postData !== null ||
      request.url.includes('browser-smoke.bin') ||
      request.url.includes(fixture.filename),
  );
  expect(userDataRequests).toEqual([]);
});

test('Inspect identifies a modified Slice and Merge refuses it', async ({ page }) => {
  const fixture = createPackageFixture();
  const [firstSlice, secondSlice] = fixture.slices;
  if (!firstSlice || !secondSlice) {
    throw new Error('Fixture must contain two slices');
  }
  const corrupted = [
    firstSlice,
    { ...secondSlice, buffer: Buffer.from('tampered') },
  ];
  await page.goto('/');
  await page.getByRole('button', { name: 'Inspect' }).click();
  await page.getByTestId('manifest-file').setInputFiles(fixture.manifest);
  await page.getByTestId('slice-files').setInputFiles(corrupted);
  await page.getByRole('button', { name: 'Inspect Package' }).click();
  await expect(page.getByRole('heading', { name: 'Package needs attention' })).toBeVisible();
  await expect(page.getByText('Incomplete result')).toBeVisible();
  await expect(page.getByText('Incomplete', { exact: true })).toBeVisible();
  await expect(page.getByText('1 damaged')).toBeVisible();

  await page.getByRole('button', { name: 'Merge' }).click();
  await page.getByTestId('manifest-file').setInputFiles(fixture.manifest);
  await page.getByTestId('slice-files').setInputFiles(corrupted);
  await page.getByRole('button', { name: 'Layer the Cake' }).click();
  await expect(page.getByText(/Resolve missing, duplicate, unexpected, and size-mismatched Slices/u)).toBeVisible();
  await expect(page.getByText('Cake rebuilt exactly.', { exact: false })).toHaveCount(0);
});

test('Inspect identifies duplicate and unexpected Slices', async ({ page }) => {
  const fixture = createPackageFixture();
  const [firstSlice, secondSlice] = fixture.slices;
  if (!firstSlice || !secondSlice) {
    throw new Error('Fixture must contain two slices');
  }
  await page.goto('/');
  await page.getByRole('button', { name: 'Inspect' }).click();
  await page.getByTestId('manifest-file').setInputFiles(fixture.manifest);
  await page.getByTestId('slice-files').setInputFiles([
    firstSlice,
    { ...firstSlice },
    secondSlice,
    {
      name: 'unexpected.001.slice',
      mimeType: 'application/octet-stream',
      buffer: Buffer.from('unexpected'),
    },
  ]);
  await page.getByRole('button', { name: 'Inspect Package' }).click();
  await expect(page.getByRole('heading', { name: 'Package needs attention' })).toBeVisible();
  await expect(page.getByText('1 duplicate')).toBeVisible();
  await expect(page.getByText('1 unexpected')).toBeVisible();
  await expect(page.locator('.ledger-note')).toContainText('unexpected.001.slice');
});

test('Compatibility Mode rejects an excessive download plan before processing', async ({ page }) => {
  await page.goto('/');
  await page.getByTestId('split-file').setInputFiles({
    name: 'too-many-slices.bin',
    mimeType: 'application/octet-stream',
    buffer: Buffer.alloc(1_001, 7),
  });
  await page.getByLabel('Target Slice size in bytes').fill('1');
  await expect(page.getByText(/1001 planned Slices/u)).toBeVisible();
  await page.getByRole('button', { name: 'Cut the Cake' }).click();
  await expect(page.getByText(/supports at most 1000 downloads/u)).toBeVisible();
  await expect(page.getByText('Processing', { exact: true })).toHaveCount(0);
});

test('Pause, resume, and cancel keep output incomplete and persist a terminal task', async ({ page }, testInfo) => {
  test.setTimeout(90_000);
  const fixture = await sparseFixture(testInfo, 'controlled-task.bin', 192 * 1024 * 1024);
  await page.goto('/');
  const session = await page.context().newCDPSession(page);
  await session.send('Emulation.setCPUThrottlingRate', { rate: 6 });
  await page.getByTestId('split-file').setInputFiles(fixture);
  await page.getByLabel('Target Slice size in bytes').fill(String(192 * 1024 * 1024));
  await page.getByRole('button', { name: 'Cut the Cake' }).click();
  await page.getByRole('button', { name: 'Pause' }).click();
  await expect(page.getByText('Paused', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Resume' }).click();
  await expect(page.getByText('Processing', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Cancel safely' }).click();
  await expect(page.getByText('Operation cancelled', { exact: true })).toBeVisible();
  await expect(page.getByText('Cancelled', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Tasks' }).click();
  await expect(page.getByRole('heading', { name: 'controlled-task.bin' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Browser-local task storage' }).getByText('Cancelled', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reselect and restart safely' })).toBeVisible();
});

test('Reload marks an active task interrupted and requires reselection', async ({ page }, testInfo) => {
  test.setTimeout(90_000);
  const fixture = await sparseFixture(testInfo, 'interrupted-task.bin', 192 * 1024 * 1024);
  await page.goto('/');
  const session = await page.context().newCDPSession(page);
  await session.send('Emulation.setCPUThrottlingRate', { rate: 6 });
  await page.getByTestId('split-file').setInputFiles(fixture);
  await page.getByLabel('Target Slice size in bytes').fill(String(192 * 1024 * 1024));
  await page.getByRole('button', { name: 'Cut the Cake' }).click();
  await expect(page.getByText('Processing', { exact: true })).toBeVisible();
  await page.reload();
  await page.getByRole('button', { name: 'Tasks' }).click();
  await expect(page.getByRole('heading', { name: 'interrupted-task.bin' })).toBeVisible();
  await expect(page.getByText('Interrupted', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Reselect and restart safely' }).click();
  await expect(page.getByText('Recovery requires reselection')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Cut the Cake' })).toBeDisabled();
  await expect(page.getByText(/restart from byte zero under a new task ID/u)).toBeVisible();
});

async function sparseFixture(testInfo: TestInfo, filename: string, size: number): Promise<string> {
  const fixture = testInfo.outputPath(filename);
  await mkdir(path.dirname(fixture), { recursive: true });
  const handle = await open(fixture, 'w');
  try {
    await handle.truncate(size);
  } finally {
    await handle.close();
  }
  return fixture;
}

function createPackageFixture() {
  const filename = 'verified-cake.bin';
  const original = Buffer.from('browser merge fixture bytes');
  const targetSliceSize = 14;
  const chunks = [original.subarray(0, targetSliceSize), original.subarray(targetSliceSize)];
  const slices = chunks.map((chunk, position) => ({
    name: `${filename}.${String(position + 1).padStart(3, '0')}.slice`,
    mimeType: 'application/octet-stream',
    buffer: Buffer.from(chunk),
  }));
  const manifest = {
    format: 'cakesplitter',
    version: '1.0',
    packageId: randomUUID(),
    createdAt: new Date('2026-07-16T04:00:00Z').toISOString(),
    original: {
      filename,
      size: original.length,
      sha256: sha256(original),
    },
    targetSliceSize,
    sliceCount: slices.length,
    slices: chunks.map((chunk, position) => ({
      index: position + 1,
      filename: slices[position]?.name,
      offset: position * targetSliceSize,
      size: chunk.length,
      sha256: sha256(chunk),
    })),
  };
  return {
    filename,
    original,
    manifest: {
      name: `${filename}.cake.json`,
      mimeType: 'application/json',
      buffer: Buffer.from(JSON.stringify(manifest)),
    },
    slices,
  };
}

function sha256(bytes: Uint8Array) {
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
