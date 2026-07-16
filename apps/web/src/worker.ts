/// <reference lib="webworker" />

import {
  FORMAT_IDENTIFIER,
  FORMAT_VERSION,
  IncrementalSha256,
  MAX_BROWSER_FALLBACK_BYTES,
  MAX_BROWSER_FALLBACK_DOWNLOADS,
  manifestFilename,
  parseManifest,
  planSlices,
  type CakeManifest,
  type SliceEntry,
} from '@cakesplitter/shared-types';
import { streamBlob } from '@cakesplitter/web-file-io';

import {
  WorkerProtocolError,
  parseWorkerRequest,
  type InspectionResult,
  type SliceVerification,
  type WorkerRequest,
  type WorkerResponse,
} from './protocol';

const worker = self as DedicatedWorkerGlobalScope;
let cancelled = false;

worker.addEventListener('message', (event: MessageEvent<unknown>) => {
  let request: WorkerRequest;
  try {
    request = parseWorkerRequest(event.data);
  } catch (error) {
    post({
      type: 'error',
      state: 'failed',
      code: errorCode(error),
      message: errorMessage(error),
    });
    return;
  }
  if (request.type === 'cancel') {
    cancelled = true;
    return;
  }
  cancelled = false;
  void dispatch(request).catch((error: unknown) => {
    const wasCancelled =
      cancelled || (error instanceof DOMException && error.name === 'AbortError');
    post({
      type: 'error',
      state: wasCancelled ? 'cancelled' : 'failed',
      code: wasCancelled ? 'cancelled' : errorCode(error),
      message: wasCancelled ? 'Operation cancelled. No incomplete output was marked complete.' : errorMessage(error),
    });
  });
});

async function dispatch(request: Exclude<WorkerRequest, { type: 'cancel' }>): Promise<void> {
  switch (request.type) {
    case 'split':
      assertFallbackOnly(request.directory);
      await splitCake(request.file, request.sliceSize);
      break;
    case 'inspect':
      await inspectCake(request.manifestText, request.files);
      break;
    case 'merge':
      assertFallbackOnly(request.directory);
      await mergeCake(request.manifestText, request.files);
      break;
  }
}

async function splitCake(file: File, sliceSize: number): Promise<void> {
  if (file.size > MAX_BROWSER_FALLBACK_BYTES) {
    throw new Error(
      `Compatibility Split is limited to ${MAX_BROWSER_FALLBACK_BYTES} bytes to bound browser memory.`,
    );
  }
  const plan = planSlices(file.name, file.size, sliceSize);
  if (plan.length > MAX_BROWSER_FALLBACK_DOWNLOADS) {
    throw new Error(
      `Compatibility Split is limited to ${MAX_BROWSER_FALLBACK_DOWNLOADS} downloaded files.`,
    );
  }
  const manifestName = manifestFilename(file.name);

  const originalHasher = new IncrementalSha256();
  const slices: SliceEntry[] = [];
  let totalProcessed = 0;
  for (const entry of plan) {
    assertNotCancelled();
    const source = file.slice(entry.offset, entry.offset + entry.size);
    const sliceHasher = new IncrementalSha256();
    const chunks: BlobPart[] = [];
    await streamBlob(
      source,
      (chunk) => {
        assertNotCancelled();
        originalHasher.update(chunk);
        sliceHasher.update(chunk);
        chunks.push(new Uint8Array(chunk).buffer);
        totalProcessed += chunk.byteLength;
        postProgress(
          'split',
          totalProcessed,
          file.size,
          entry.index,
          plan.length,
          `Cutting Slice ${entry.index} of ${plan.length}`,
        );
      },
      () => cancelled,
    );
    slices.push({
      ...entry,
      sha256: sliceHasher.digestHex(),
    });
    post({ type: 'download', filename: entry.filename, blob: new Blob(chunks) });
  }

  const manifest: CakeManifest = {
    format: FORMAT_IDENTIFIER,
    version: FORMAT_VERSION,
    packageId: crypto.randomUUID(),
    createdAt: new Date().toISOString(),
    original: {
      filename: file.name,
      size: file.size,
      sha256: originalHasher.digestHex(),
    },
    targetSliceSize: sliceSize,
    sliceCount: slices.length,
    slices,
  };
  parseManifest(JSON.stringify(manifest));
  const manifestBlob = new Blob([`${JSON.stringify(manifest, null, 2)}\n`], {
    type: 'application/json',
  });

  post({ type: 'download', filename: manifestName, blob: manifestBlob });
  post({
    type: 'result',
    operation: 'split',
    mode: 'fallback',
    message: `Cake cut into ${slices.length} verified ${slices.length === 1 ? 'Slice' : 'Slices'}.`,
    manifest,
  });
}

async function inspectCake(manifestText: string, files: File[]): Promise<void> {
  const manifest = parseManifest(manifestText);
  const inspection = await verifySelectedFiles(manifest, files, 'inspect');
  post({
    type: 'result',
    operation: 'inspect',
    mode: 'read-only',
    message: inspection.verified
      ? 'Every expected Slice is present and verified.'
      : 'Inspection found package issues. Review the Slice ledger below.',
    inspection,
  });
}

async function mergeCake(manifestText: string, files: File[]): Promise<void> {
  const manifest = parseManifest(manifestText);
  if (manifest.original.size > MAX_BROWSER_FALLBACK_BYTES) {
    throw new Error(
      `Compatibility Merge is limited to ${MAX_BROWSER_FALLBACK_BYTES} bytes to bound browser memory.`,
    );
  }
  const selection = indexSelectedFiles(manifest, files);
  if (
    selection.missing.length > 0 ||
    selection.duplicates.length > 0 ||
    selection.unexpected.length > 0
  ) {
    throw new Error(
      `Package selection is not complete (missing ${selection.missing.length}, duplicate ${selection.duplicates.length}, unexpected ${selection.unexpected.length}).`,
    );
  }
  const outputChunks: BlobPart[] = [];
  const originalHasher = new IncrementalSha256();
  let bytesProcessed = 0;
  for (const entry of manifest.slices) {
    assertNotCancelled();
    const file = selection.byName.get(entry.filename)?.[0];
    if (!file) {
      throw new Error(`Missing Slice: ${entry.filename}`);
    }
    if (file.size !== entry.size) {
      throw new Error(`Damaged Slice size: ${entry.filename}`);
    }
    const sliceHasher = new IncrementalSha256();
    await streamBlob(
      file,
      (chunk) => {
        sliceHasher.update(chunk);
        originalHasher.update(chunk);
        outputChunks.push(new Uint8Array(chunk).buffer);
        bytesProcessed += chunk.byteLength;
        postProgress(
          'merge',
          bytesProcessed,
          manifest.original.size,
          entry.index,
          manifest.sliceCount,
          `Layering Slice ${entry.index} of ${manifest.sliceCount}`,
        );
      },
      () => cancelled,
    );
    if (sliceHasher.digestHex() !== entry.sha256) {
      throw new Error(`Damaged Slice hash: ${entry.filename}`);
    }
  }
  const outputSha256 = originalHasher.digestHex();
  if (outputSha256 !== manifest.original.sha256) {
    throw new Error('Rebuilt Cake hash does not match the manifest.');
  }
  post({
    type: 'download',
    filename: manifest.original.filename,
    blob: new Blob(outputChunks),
  });
  post({
    type: 'result',
    operation: 'merge',
    mode: 'fallback',
    message: 'Cake rebuilt exactly. The final SHA-256 matches the manifest.',
    outputFilename: manifest.original.filename,
    outputSha256,
  });
}

async function verifySelectedFiles(
  manifest: CakeManifest,
  files: File[],
  operation: 'inspect',
): Promise<InspectionResult> {
  const selection = indexSelectedFiles(manifest, files);
  const corrupted: string[] = [];
  const slices: SliceVerification[] = [];
  let bytesProcessed = 0;
  for (const entry of manifest.slices) {
    assertNotCancelled();
    const matches = selection.byName.get(entry.filename) ?? [];
    if (matches.length === 0) {
      slices.push({
        index: entry.index,
        filename: entry.filename,
        state: 'missing',
        detail: 'Expected by the manifest but not selected.',
      });
      continue;
    }
    if (matches.length > 1) {
      slices.push({
        index: entry.index,
        filename: entry.filename,
        state: 'duplicate',
        detail: `${matches.length} files share this expected name.`,
      });
      continue;
    }
    const file = matches[0];
    if (!file || file.size !== entry.size) {
      corrupted.push(entry.filename);
      slices.push({
        index: entry.index,
        filename: entry.filename,
        state: 'corrupted',
        detail: `Expected ${formatBytes(entry.size)}; selected file has ${formatBytes(file?.size ?? 0)}.`,
      });
      continue;
    }
    const hasher = new IncrementalSha256();
    await streamBlob(
      file,
      (chunk) => {
        hasher.update(chunk);
        bytesProcessed += chunk.byteLength;
        postProgress(
          operation,
          bytesProcessed,
          manifest.original.size,
          entry.index,
          manifest.sliceCount,
          `Verifying Slice ${entry.index} of ${manifest.sliceCount}`,
        );
      },
      () => cancelled,
    );
    if (hasher.digestHex() !== entry.sha256) {
      corrupted.push(entry.filename);
      slices.push({
        index: entry.index,
        filename: entry.filename,
        state: 'corrupted',
        detail: 'SHA-256 does not match the manifest.',
      });
    } else {
      slices.push({
        index: entry.index,
        filename: entry.filename,
        state: 'verified',
        detail: 'Size and SHA-256 match.',
      });
    }
  }
  return {
    manifest,
    foundSliceCount: files.filter((file) => manifest.slices.some((entry) => entry.filename === file.name)).length,
    missing: selection.missing,
    corrupted,
    duplicates: selection.duplicates,
    unexpected: selection.unexpected,
    verified:
      selection.missing.length === 0 &&
      corrupted.length === 0 &&
      selection.duplicates.length === 0 &&
      selection.unexpected.length === 0,
    slices,
  };
}

function indexSelectedFiles(manifest: CakeManifest, files: File[]) {
  const expected = new Set(manifest.slices.map((entry) => entry.filename));
  const byName = new Map<string, File[]>();
  for (const file of files) {
    const matches = byName.get(file.name) ?? [];
    matches.push(file);
    byName.set(file.name, matches);
  }
  const missing = manifest.slices
    .filter((entry) => !byName.has(entry.filename))
    .map((entry) => entry.filename);
  const duplicates = [...byName.entries()]
    .filter(([name, matches]) => expected.has(name) && matches.length > 1)
    .map(([name]) => name);
  const unexpected = files
    .filter((file) => file.name.endsWith('.slice') && !expected.has(file.name))
    .map((file) => file.name)
    .sort();
  return { byName, missing, duplicates, unexpected };
}

function assertFallbackOnly(directory: FileSystemDirectoryHandle | undefined): void {
  if (directory) {
    throw new WorkerProtocolError(
      'Direct-folder output is disabled because browser APIs cannot guarantee exclusive creation, no-replace publication, and ownership-safe cleanup. Use compatibility download mode.',
    );
  }
}

function postProgress(
  operation: 'split' | 'merge' | 'inspect',
  bytesProcessed: number,
  totalBytes: number,
  currentSlice: number,
  sliceCount: number,
  message: string,
) {
  post({
    type: 'progress',
    operation,
    bytesProcessed,
    totalBytes,
    currentSlice,
    sliceCount,
    message,
  });
}

function post(message: WorkerResponse): void {
  worker.postMessage(message);
}

function assertNotCancelled(): void {
  if (cancelled) {
    throw new DOMException('Operation cancelled', 'AbortError');
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : 'Unknown processing error';
}

function errorCode(error: unknown): string {
  if (error instanceof WorkerProtocolError) {
    return error.code;
  }
  if (error instanceof Error && /already exists/i.test(error.message)) {
    return 'output_collision';
  }
  if (error instanceof Error && /manifest|slice|package/i.test(error.message)) {
    return 'invalid_package';
  }
  return 'processing_failed';
}

function formatBytes(bytes: number): string {
  return `${new Intl.NumberFormat().format(bytes)} bytes`;
}

export {};
