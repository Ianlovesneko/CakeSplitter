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
import {
  DirectFolderSecurityError,
  getDirectFolderCapabilities,
  streamBlob,
} from '@cakesplitter/web-file-io';

import {
  WorkerProtocolError,
  parseWorkerRequest,
  type InspectionResult,
  type SliceVerification,
  type TaskStatus,
  type WorkerOperation,
  type WorkerRequest,
  type WorkerResponse,
} from './protocol';
import { classifyStart, WorkerTaskControl } from './worker-control';

const worker = self as DedicatedWorkerGlobalScope;

interface ActiveTask {
  requestId: string;
  taskId: string;
  operation: WorkerOperation;
  startedAt: number;
  control: WorkerTaskControl;
}

let activeTask: ActiveTask | undefined;

worker.addEventListener('message', (event: MessageEvent<unknown>) => {
  let request: WorkerRequest;
  try {
    request = parseWorkerRequest(event.data);
  } catch (error) {
    const fallback = messageFallbackIdentity(event.data);
    post({
      type: 'error',
      ...fallback,
      operation: fallback.operation,
      status: 'failed',
      code: errorCode(error),
      message: errorMessage(error),
    });
    return;
  }

  if (request.type === 'control') {
    handleControl(request);
    return;
  }
  const admission = classifyStart(activeTask?.taskId, request.taskId);
  if (admission !== 'accept') {
    post({
      type: 'error',
      requestId: request.requestId,
      taskId: request.taskId,
      operation: request.operation,
      status: 'failed',
      code: admission,
      message:
        admission === 'duplicate_task_id'
          ? 'This task ID is already active.'
          : 'Another task is active. Wait for a terminal state before starting a new task.',
    });
    return;
  }

  const task: ActiveTask = {
    requestId: request.requestId,
    taskId: request.taskId,
    operation: request.operation,
    startedAt: performance.now(),
    control: new WorkerTaskControl(),
  };
  activeTask = task;
  postState(task, 'running', 'Task accepted by the processing Worker.');
  void dispatch(request, task)
    .catch((error: unknown) => {
      const status = errorStatus(error, task);
      post({
        type: 'error',
        requestId: task.requestId,
        taskId: task.taskId,
        operation: task.operation,
        status,
        code: status === 'cancelled' ? 'cancelled' : errorCode(error),
        message:
          status === 'cancelled'
            ? 'Operation cancelled. Incomplete output was not marked verified.'
            : errorMessage(error),
      });
    })
    .finally(() => {
      if (activeTask === task) activeTask = undefined;
      task.control.release();
    });
});

function handleControl(request: Extract<WorkerRequest, { type: 'control' }>): void {
  const task = activeTask;
  if (!task || task.taskId !== request.taskId) {
    post({
      type: 'error',
      requestId: request.requestId,
      taskId: request.taskId,
      operation: task?.operation ?? 'inspect',
      status: 'failed',
      code: 'stale_task_message',
      message: 'The control message does not belong to the active task.',
    });
    return;
  }
  switch (request.command) {
    case 'cancel':
      task.control.apply('cancel');
      break;
    case 'pause':
      if (task.control.apply('pause') === 'paused') {
        postState(task, 'paused', 'Paused at a bounded chunk boundary. Output remains incomplete.');
      }
      break;
    case 'resume':
      if (task.control.apply('resume') === 'running') {
        postState(task, 'running', 'Task resumed after rechecking active task state.');
      }
      break;
  }
}

async function dispatch(
  request: Extract<WorkerRequest, { type: 'start' }>,
  task: ActiveTask,
): Promise<void> {
  if (request.operation !== task.operation || request.taskId !== task.taskId) {
    throw new WorkerProtocolError('Worker dispatch identity mismatch');
  }
  switch (request.operation) {
    case 'split':
      assertOutputModeAvailable(request.outputMode);
      await splitCake(request.file, request.sliceSize, task);
      break;
    case 'inspect':
      await inspectCake(request.manifestText, request.files, task);
      break;
    case 'merge':
      assertOutputModeAvailable(request.outputMode);
      await mergeCake(request.manifestText, request.files, task);
      break;
  }
}

function assertOutputModeAvailable(mode: 'direct' | 'fallback'): void {
  if (mode === 'direct') {
    const capabilities = getDirectFolderCapabilities(worker);
    throw new DirectFolderSecurityError(
      'unsupported_finalization',
      `Direct Folder Mode remains fail-closed: ${capabilities.reason}`,
    );
  }
}

async function splitCake(file: File, sliceSize: number, task: ActiveTask): Promise<void> {
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
  const originalHasher = new IncrementalSha256();
  const slices: SliceEntry[] = [];
  let totalProcessed = 0;
  for (const entry of plan) {
    await checkpoint(task);
    const source = file.slice(entry.offset, entry.offset + entry.size);
    const sliceHasher = new IncrementalSha256();
    const chunks: BlobPart[] = [];
    await streamBlob(
      source,
      async (chunk) => {
        await checkpoint(task);
        originalHasher.update(chunk);
        sliceHasher.update(chunk);
        chunks.push(new Uint8Array(chunk).buffer);
        totalProcessed += chunk.byteLength;
        postProgress(task, totalProcessed, file.size, entry.index, plan.length, `Cutting Slice ${entry.index} of ${plan.length}`);
      },
      () => task.control.cancelled,
      undefined,
      () => checkpoint(task),
    );
    slices.push({ ...entry, sha256: sliceHasher.digestHex() });
    post({
      type: 'download',
      requestId: task.requestId,
      taskId: task.taskId,
      operation: 'split',
      filename: entry.filename,
      blob: new Blob(chunks),
    });
  }

  const manifest: CakeManifest = {
    format: FORMAT_IDENTIFIER,
    version: FORMAT_VERSION,
    packageId: crypto.randomUUID(),
    createdAt: new Date().toISOString(),
    original: { filename: file.name, size: file.size, sha256: originalHasher.digestHex() },
    targetSliceSize: sliceSize,
    sliceCount: slices.length,
    slices,
  };
  parseManifest(JSON.stringify(manifest));
  post({
    type: 'download',
    requestId: task.requestId,
    taskId: task.taskId,
    operation: 'split',
    filename: manifestFilename(file.name),
    blob: new Blob([`${JSON.stringify(manifest, null, 2)}\n`], { type: 'application/json' }),
  });
  post({
    type: 'result',
    requestId: task.requestId,
    taskId: task.taskId,
    operation: 'split',
    status: 'completed',
    mode: 'fallback',
    message: `Cake cut into ${slices.length} verified ${slices.length === 1 ? 'Slice' : 'Slices'}.`,
    manifest,
  });
}

async function inspectCake(
  manifestText: string,
  files: File[],
  task: ActiveTask,
): Promise<void> {
  const manifest = parseManifest(manifestText);
  const inspection = await verifySelectedFiles(manifest, files, task);
  post({
    type: 'result',
    requestId: task.requestId,
    taskId: task.taskId,
    operation: 'inspect',
    status: inspection.verified ? 'completed' : 'incomplete',
    mode: 'read-only',
    message: inspection.verified
      ? 'Every expected Slice is present and verified.'
      : 'Inspection found package issues. Review the Slice ledger below.',
    inspection,
  });
}

async function mergeCake(
  manifestText: string,
  files: File[],
  task: ActiveTask,
): Promise<void> {
  const manifest = parseManifest(manifestText);
  if (manifest.original.size > MAX_BROWSER_FALLBACK_BYTES) {
    throw new Error(
      `Compatibility Merge is limited to ${MAX_BROWSER_FALLBACK_BYTES} bytes to bound browser memory.`,
    );
  }
  const selection = indexSelectedFiles(manifest, files);
  if (selection.missing.length || selection.duplicates.length || selection.unexpected.length) {
    throw new Error(
      `Package selection is not complete (missing ${selection.missing.length}, duplicate ${selection.duplicates.length}, unexpected ${selection.unexpected.length}).`,
    );
  }
  const outputChunks: BlobPart[] = [];
  const originalHasher = new IncrementalSha256();
  let bytesProcessed = 0;
  for (const entry of manifest.slices) {
    await checkpoint(task);
    const file = selection.byName.get(entry.filename)?.[0];
    if (!file) throw new Error(`Missing Slice: ${entry.filename}`);
    if (file.size !== entry.size) throw new Error(`Damaged Slice size: ${entry.filename}`);
    const sliceHasher = new IncrementalSha256();
    await streamBlob(
      file,
      async (chunk) => {
        await checkpoint(task);
        sliceHasher.update(chunk);
        originalHasher.update(chunk);
        outputChunks.push(new Uint8Array(chunk).buffer);
        bytesProcessed += chunk.byteLength;
        postProgress(task, bytesProcessed, manifest.original.size, entry.index, manifest.sliceCount, `Layering Slice ${entry.index} of ${manifest.sliceCount}`);
      },
      () => task.control.cancelled,
      undefined,
      () => checkpoint(task),
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
    requestId: task.requestId,
    taskId: task.taskId,
    operation: 'merge',
    filename: manifest.original.filename,
    blob: new Blob(outputChunks),
  });
  post({
    type: 'result',
    requestId: task.requestId,
    taskId: task.taskId,
    operation: 'merge',
    status: 'completed',
    mode: 'fallback',
    message: 'Cake rebuilt exactly. The final SHA-256 matches the manifest.',
    outputFilename: manifest.original.filename,
    outputSha256,
  });
}

async function verifySelectedFiles(
  manifest: CakeManifest,
  files: File[],
  task: ActiveTask,
): Promise<InspectionResult> {
  const selection = indexSelectedFiles(manifest, files);
  const corrupted: string[] = [];
  const slices: SliceVerification[] = [];
  let bytesProcessed = 0;
  for (const entry of manifest.slices) {
    await checkpoint(task);
    const matches = selection.byName.get(entry.filename) ?? [];
    if (matches.length === 0) {
      slices.push({ index: entry.index, filename: entry.filename, state: 'missing', detail: 'Expected by the manifest but not selected.' });
      continue;
    }
    if (matches.length > 1) {
      slices.push({ index: entry.index, filename: entry.filename, state: 'duplicate', detail: `${matches.length} files share this expected name.` });
      continue;
    }
    const file = matches[0];
    if (!file || file.size !== entry.size) {
      corrupted.push(entry.filename);
      slices.push({ index: entry.index, filename: entry.filename, state: 'corrupted', detail: `Expected ${formatBytes(entry.size)}; selected file has ${formatBytes(file?.size ?? 0)}.` });
      continue;
    }
    const hasher = new IncrementalSha256();
    await streamBlob(
      file,
      async (chunk) => {
        await checkpoint(task);
        hasher.update(chunk);
        bytesProcessed += chunk.byteLength;
        postProgress(task, bytesProcessed, manifest.original.size, entry.index, manifest.sliceCount, `Verifying Slice ${entry.index} of ${manifest.sliceCount}`);
      },
      () => task.control.cancelled,
      undefined,
      () => checkpoint(task),
    );
    if (hasher.digestHex() !== entry.sha256) {
      corrupted.push(entry.filename);
      slices.push({ index: entry.index, filename: entry.filename, state: 'corrupted', detail: 'SHA-256 does not match the manifest.' });
    } else {
      slices.push({ index: entry.index, filename: entry.filename, state: 'verified', detail: 'Size and SHA-256 match.' });
    }
  }
  return {
    manifest,
    foundSliceCount: files.filter((file) => manifest.slices.some((entry) => entry.filename === file.name)).length,
    missing: selection.missing,
    corrupted,
    duplicates: selection.duplicates,
    unexpected: selection.unexpected,
    verified: !selection.missing.length && !corrupted.length && !selection.duplicates.length && !selection.unexpected.length,
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
  const missing = manifest.slices.filter((entry) => !byName.has(entry.filename)).map((entry) => entry.filename);
  const duplicates = [...byName.entries()].filter(([name, matches]) => expected.has(name) && matches.length > 1).map(([name]) => name);
  const unexpected = files.filter((file) => file.name.endsWith('.slice') && !expected.has(file.name)).map((file) => file.name).sort();
  return { byName, missing, duplicates, unexpected };
}

async function checkpoint(task: ActiveTask): Promise<void> {
  await task.control.checkpoint(() => activeTask === task);
}

function postProgress(
  task: ActiveTask,
  bytesProcessed: number,
  totalBytes: number,
  currentSlice: number,
  sliceCount: number,
  message: string,
): void {
  const elapsedSeconds = Math.max((performance.now() - task.startedAt) / 1_000, 0.001);
  post({
    type: 'progress',
    requestId: task.requestId,
    taskId: task.taskId,
    operation: task.operation,
    status: task.control.paused ? 'paused' : 'running',
    bytesProcessed,
    totalBytes,
    currentSlice,
    sliceCount,
    speedBytesPerSecond: Math.floor(bytesProcessed / elapsedSeconds),
    message,
  });
}

function postState(task: ActiveTask, status: TaskStatus, message: string): void {
  post({
    type: 'state',
    requestId: task.requestId,
    taskId: task.taskId,
    operation: task.operation,
    status,
    message,
  });
}

function post(message: WorkerResponse): void {
  worker.postMessage(message);
}

function messageFallbackIdentity(value: unknown): {
  requestId: string;
  taskId: string;
  operation: WorkerOperation;
} {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const record = value as Record<string, unknown>;
    return {
      requestId: safeIdentifier(record.requestId, 'invalid-request'),
      taskId: safeIdentifier(record.taskId, 'invalid-task'),
      operation:
        record.operation === 'split' || record.operation === 'merge' || record.operation === 'inspect'
          ? record.operation
          : 'inspect',
    };
  }
  return { requestId: 'invalid-request', taskId: 'invalid-task', operation: 'inspect' };
}

function safeIdentifier(value: unknown, fallback: string): string {
  return typeof value === 'string' && /^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/u.test(value)
    ? value
    : fallback;
}

function errorStatus(error: unknown, task: ActiveTask): Extract<TaskStatus, 'cancelled' | 'failed' | 'permission-required' | 'incomplete'> {
  if (task.control.cancelled || (error instanceof DOMException && error.name === 'AbortError')) return 'cancelled';
  if (error instanceof DOMException && error.name === 'NotAllowedError') return 'permission-required';
  if (error instanceof DirectFolderSecurityError && error.code !== 'unsupported_finalization') return 'incomplete';
  return 'failed';
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : 'Unknown processing error';
}

function errorCode(error: unknown): string {
  if (error instanceof WorkerProtocolError || error instanceof DirectFolderSecurityError) return error.code;
  if (error instanceof DOMException && error.name === 'NotAllowedError') return 'permission_revoked';
  if (error instanceof Error && /already exists|collision/iu.test(error.message)) return 'output_collision';
  if (error instanceof Error && /manifest|slice|package/iu.test(error.message)) return 'invalid_package';
  return 'processing_failed';
}

function formatBytes(bytes: number): string {
  return `${new Intl.NumberFormat().format(bytes)} bytes`;
}

export {};
