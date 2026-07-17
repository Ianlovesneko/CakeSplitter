import { expect, test } from '@playwright/test';

test('Clear All fences active Split and Merge persistence in real OPFS', async ({ page }) => {
  await page.addInitScript(() => {
    class ControlledWorker extends EventTarget {
      private startRequest: Record<string, unknown> | undefined;

      postMessage(message: Record<string, unknown>) {
        if (message.type === 'start') {
          this.startRequest = message;
          queueMicrotask(() => {
            this.dispatchEvent(new MessageEvent('message', {
              data: {
                type: 'state',
                requestId: message.requestId,
                taskId: message.taskId,
                operation: message.operation,
                status: 'running',
                message: 'Controlled task is active.',
              },
            }));
          });
          return;
        }
        if (message.type !== 'control' || message.command !== 'cancel' || !this.startRequest) return;
        const active = this.startRequest;
        queueMicrotask(() => {
          this.dispatchEvent(new MessageEvent('message', {
            data: {
              type: 'error',
              requestId: active.requestId,
              taskId: active.taskId,
              operation: active.operation,
              status: 'cancelled',
              code: 'cancelled',
              message: 'Controlled task acknowledged cancellation.',
            },
          }));
          setTimeout(() => {
            this.dispatchEvent(new MessageEvent('message', {
              data: {
                type: 'progress',
                requestId: active.requestId,
                taskId: active.taskId,
                operation: active.operation,
                status: 'running',
                bytesProcessed: 1,
                totalBytes: 1,
                currentSlice: 1,
                sliceCount: 1,
                speedBytesPerSecond: 1,
                message: 'Stale progress after Clear All.',
              },
            }));
          }, 25);
        });
      }

      terminate() {}
    }

    Object.defineProperty(globalThis, 'Worker', {
      configurable: true,
      value: ControlledWorker,
    });
  });

  await page.goto('/');
  await clearOpfsRoot(page);

  await page.getByTestId('split-file').setInputFiles({
    name: 'active-split.bin',
    mimeType: 'application/octet-stream',
    buffer: Buffer.from('split bytes'),
  });
  await page.getByLabel('Target Slice size in bytes').fill('64');
  await page.getByRole('button', { name: 'Cut the Cake' }).click();
  await expect(page.getByRole('button', { name: 'Pause' })).toBeVisible();

  await page.getByRole('button', { name: 'Tasks' }).click();
  acceptNextDialog(page);
  await page.getByRole('button', { name: 'Clear all local data' }).click();
  await expect(page.getByText('All browser-local CakeSplitter task metadata was cleared.')).toBeVisible();
  await expect(page.getByText('No stored tasks')).toBeVisible();
  await page.waitForTimeout(75);
  expect(await opfsEntryNames(page)).toEqual([]);

  await page.getByRole('button', { name: 'Merge' }).click();
  await page.getByTestId('manifest-file').setInputFiles({
    name: 'empty.bin.cake.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify({
      format: 'cakesplitter',
      version: '1.0',
      packageId: 'ff7cb026-f7ec-4d17-a3e4-8083217ec688',
      createdAt: '2026-07-16T04:00:00.000Z',
      original: {
        filename: 'empty.bin',
        size: 0,
        sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
      },
      targetSliceSize: 1_048_576,
      sliceCount: 0,
      slices: [],
    })),
  });
  await page.getByRole('button', { name: 'Layer the Cake' }).click();
  await expect(page.getByRole('button', { name: 'Pause' })).toBeVisible();

  await page.getByRole('button', { name: 'Tasks' }).click();
  await expect(page.getByRole('heading', { name: 'empty.bin' })).toBeVisible();
  acceptNextDialog(page);
  await page.getByRole('button', { name: 'Clear all local data' }).click();
  await expect(page.getByText('All browser-local CakeSplitter task metadata was cleared.')).toBeVisible();
  await page.waitForTimeout(75);
  expect(await opfsEntryNames(page)).toEqual([]);

  await page.reload();
  await page.getByRole('button', { name: 'Tasks' }).click();
  await expect(page.getByText('No stored tasks')).toBeVisible();
  expect(await opfsEntryNames(page)).toEqual([]);
});

function acceptNextDialog(page: import('@playwright/test').Page) {
  page.once('dialog', async (dialog) => dialog.accept());
}

async function clearOpfsRoot(page: import('@playwright/test').Page) {
  await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    for await (const entry of (root as FileSystemDirectoryHandle & {
      values(): AsyncIterable<{ name: string }>;
    }).values()) {
      await root.removeEntry(entry.name, { recursive: true });
    }
  });
}

async function opfsEntryNames(page: import('@playwright/test').Page): Promise<string[]> {
  return page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const names: string[] = [];
    for await (const entry of (root as FileSystemDirectoryHandle & {
      values(): AsyncIterable<{ name: string }>;
    }).values()) {
      names.push(entry.name);
    }
    return names.sort();
  });
}
