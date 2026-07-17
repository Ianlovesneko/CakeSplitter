import {
  IncrementalSha256,
  validatePortableFilename,
} from '@cakesplitter/shared-types';

export const STREAM_CHUNK_SIZE = 1024 * 1024;
export const DIRECT_FOLDER_MODE_ENABLED = false;

export interface StreamProgress {
  bytesRead: number;
  totalBytes: number;
}

export interface DirectFolderCapabilities {
  secureContext: boolean;
  openFilePicker: boolean;
  directoryPicker: boolean;
  fileHandle: boolean;
  directoryHandle: boolean;
  writableStream: boolean;
  handleIdentity: boolean;
  move: boolean;
  atomicNoReplace: boolean;
  supported: boolean;
  reason: string;
}

export interface StreamControl {
  checkpoint?: () => void | Promise<void>;
  onProgress?: (progress: StreamProgress) => void;
}

export interface OutputSnapshot {
  identity: string;
  size: number;
  sha256: string;
}

export interface StagedOutput {
  readonly name: string;
  readonly identity: string;
  write(chunk: Uint8Array): Promise<void>;
  close(): Promise<void>;
  abort(reason?: unknown): Promise<void>;
}

/**
 * Security boundary for direct-folder publication.
 *
 * Browser adapters must not implement this interface unless publishNoReplace is
 * a single atomic operation that fails when finalName already exists. A
 * preflight existence check followed by an overwriting move is not sufficient.
 */
export interface SecureOutputAdapter {
  readonly atomicNoReplace: true;
  directoryIdentity(): Promise<string>;
  destinationExists(name: string): Promise<boolean>;
  createPartialExclusive(name: string): Promise<StagedOutput>;
  currentIdentity(name: string): Promise<string | undefined>;
  snapshot(name: string): Promise<OutputSnapshot>;
  publishNoReplace(partialName: string, finalName: string): Promise<void>;
}

export interface VerifiedOutputRequest {
  taskId: string;
  finalName: string;
  expectedSize: number;
  expectedSha256: string;
  chunks: AsyncIterable<Uint8Array>;
  checkpoint?: () => void | Promise<void>;
  onProgress?: (bytesWritten: number) => void;
}

export interface VerifiedOutputResult {
  finalName: string;
  size: number;
  sha256: string;
}

export class DirectFolderSecurityError extends Error {
  constructor(
    readonly code:
      | 'unsupported_finalization'
      | 'output_collision'
      | 'directory_rebound'
      | 'partial_rebound'
      | 'size_mismatch'
      | 'checksum_mismatch'
      | 'final_identity_mismatch'
      | 'cancelled'
      | 'write_failed',
    message: string,
  ) {
    super(message);
    this.name = 'DirectFolderSecurityError';
  }
}

export function getDirectFolderCapabilities(scope: unknown = globalThis): DirectFolderCapabilities {
  const candidate = isRecord(scope) ? scope : {};
  const fileHandlePrototype = constructorPrototype(candidate.FileSystemFileHandle);
  const directoryHandlePrototype = constructorPrototype(candidate.FileSystemDirectoryHandle);
  const writablePrototype = constructorPrototype(candidate.FileSystemWritableFileStream);
  const secureContext = candidate.isSecureContext === true;
  const openFilePicker = typeof candidate.showOpenFilePicker === 'function';
  const directoryPicker = typeof candidate.showDirectoryPicker === 'function';
  const fileHandle = fileHandlePrototype !== undefined;
  const directoryHandle = directoryHandlePrototype !== undefined;
  const writableStream =
    writablePrototype !== undefined || typeof fileHandlePrototype?.createWritable === 'function';
  const handleIdentity =
    typeof fileHandlePrototype?.isSameEntry === 'function' &&
    typeof directoryHandlePrototype?.isSameEntry === 'function';
  const move = typeof fileHandlePrototype?.move === 'function';

  // The standardized browser surface has no atomic "move only if absent"
  // option. Chromium's move may overwrite a writable destination, so a
  // preflight check cannot close the race. Keep production fail-closed.
  const atomicNoReplace = false;
  const supported =
    DIRECT_FOLDER_MODE_ENABLED &&
    secureContext &&
    openFilePicker &&
    directoryPicker &&
    fileHandle &&
    directoryHandle &&
    writableStream &&
    handleIdentity &&
    move &&
    atomicNoReplace;

  return {
    secureContext,
    openFilePicker,
    directoryPicker,
    fileHandle,
    directoryHandle,
    writableStream,
    handleIdentity,
    move,
    atomicNoReplace,
    supported,
    reason: supported
      ? 'Secure direct-folder streaming is available.'
      : atomicNoReplace
        ? 'This browser is missing a required File System Access capability.'
        : 'This browser does not expose atomic no-replace finalization for user-selected folders.',
  };
}

export function supportsDirectFolderMode(scope?: unknown): boolean {
  return getDirectFolderCapabilities(scope).supported;
}

export async function streamBlob(
  blob: Blob,
  onChunk: (chunk: Uint8Array) => void | Promise<void>,
  isCancelled: () => boolean = () => false,
  onProgress?: (progress: StreamProgress) => void,
  checkpoint?: () => void | Promise<void>,
): Promise<void> {
  let bytesRead = 0;
  while (bytesRead < blob.size) {
    if (isCancelled()) {
      throw new DOMException('Operation cancelled', 'AbortError');
    }
    await checkpoint?.();
    const end = Math.min(bytesRead + STREAM_CHUNK_SIZE, blob.size);
    const chunk = new Uint8Array(await blob.slice(bytesRead, end).arrayBuffer());
    await onChunk(chunk);
    bytesRead += chunk.byteLength;
    onProgress?.({ bytesRead, totalBytes: blob.size });
  }
  if (blob.size === 0) {
    await checkpoint?.();
    onProgress?.({ bytesRead: 0, totalBytes: 0 });
  }
}

export async function streamBlobToWritable(
  blob: Blob,
  writable: Pick<WritableStreamDefaultWriter<Uint8Array>, 'write' | 'close' | 'abort'>,
  control: StreamControl = {},
): Promise<{ size: number; sha256: string }> {
  const hasher = new IncrementalSha256();
  let bytesWritten = 0;
  try {
    await streamBlob(
      blob,
      async (chunk) => {
        await control.checkpoint?.();
        await writable.write(chunk);
        hasher.update(chunk);
        bytesWritten += chunk.byteLength;
      },
      () => false,
      control.onProgress,
      control.checkpoint,
    );
    await writable.close();
  } catch (error) {
    await writable.abort(error).catch(() => undefined);
    throw error;
  }
  return { size: bytesWritten, sha256: hasher.digestHex() };
}

export async function hashBlob(
  blob: Blob,
  isCancelled?: () => boolean,
  onProgress?: (progress: StreamProgress) => void,
  checkpoint?: () => void | Promise<void>,
): Promise<string> {
  const hasher = new IncrementalSha256();
  await streamBlob(
    blob,
    (chunk) => {
      hasher.update(chunk);
    },
    isCancelled,
    onProgress,
    checkpoint,
  );
  return hasher.digestHex();
}

export function validateOutputPlan(names: readonly string[]): void {
  const seen = new Set<string>();
  for (const name of names) {
    validatePortableFilename(name);
    const folded = name.toLocaleLowerCase('en-US');
    if (seen.has(folded)) {
      throw new DirectFolderSecurityError(
        'output_collision',
        `Output plan contains a duplicate filename: ${name}`,
      );
    }
    seen.add(folded);
  }
}

export async function writeVerifiedOutput(
  adapter: SecureOutputAdapter,
  request: VerifiedOutputRequest,
): Promise<VerifiedOutputResult> {
  if (adapter.atomicNoReplace !== true) {
    throw new DirectFolderSecurityError(
      'unsupported_finalization',
      'Direct Folder Mode requires an atomic no-replace publication primitive.',
    );
  }
  validateOutputPlan([request.finalName]);
  if (!Number.isSafeInteger(request.expectedSize) || request.expectedSize < 0) {
    throw new DirectFolderSecurityError('size_mismatch', 'Expected output size is invalid.');
  }
  if (!/^[a-f0-9]{64}$/u.test(request.expectedSha256)) {
    throw new DirectFolderSecurityError('checksum_mismatch', 'Expected output SHA-256 is invalid.');
  }

  const directoryIdentity = await adapter.directoryIdentity();
  if (await adapter.destinationExists(request.finalName)) {
    throw new DirectFolderSecurityError(
      'output_collision',
      `Destination already exists: ${request.finalName}`,
    );
  }

  const partialName = partialFilename(request.taskId);
  const partial = await adapter.createPartialExclusive(partialName);
  const hasher = new IncrementalSha256();
  let bytesWritten = 0;
  let closed = false;
  try {
    for await (const chunk of request.chunks) {
      await request.checkpoint?.();
      await partial.write(chunk);
      hasher.update(chunk);
      bytesWritten += chunk.byteLength;
      request.onProgress?.(bytesWritten);
    }
    await partial.close();
    closed = true;
  } catch (error) {
    if (!closed) {
      await partial.abort(error).catch(() => undefined);
    }
    if (error instanceof DOMException && error.name === 'AbortError') {
      throw new DirectFolderSecurityError(
        'cancelled',
        `Operation cancelled; incomplete output remains as ${partialName}.`,
      );
    }
    throw new DirectFolderSecurityError(
      'write_failed',
      `Output write failed; incomplete output remains as ${partialName}.`,
    );
  }

  if ((await adapter.directoryIdentity()) !== directoryIdentity) {
    throw new DirectFolderSecurityError(
      'directory_rebound',
      `Output directory identity changed; ${partialName} was not published.`,
    );
  }
  if ((await adapter.currentIdentity(partialName)) !== partial.identity) {
    throw new DirectFolderSecurityError(
      'partial_rebound',
      `Incomplete output identity changed; ${partialName} was not published.`,
    );
  }

  const digest = hasher.digestHex();
  const staged = await adapter.snapshot(partialName);
  if (bytesWritten !== request.expectedSize || staged.size !== request.expectedSize) {
    throw new DirectFolderSecurityError(
      'size_mismatch',
      `Incomplete output size mismatch; ${partialName} was not published.`,
    );
  }
  if (digest !== request.expectedSha256 || staged.sha256 !== request.expectedSha256) {
    throw new DirectFolderSecurityError(
      'checksum_mismatch',
      `Incomplete output checksum mismatch; ${partialName} was not published.`,
    );
  }
  if (await adapter.destinationExists(request.finalName)) {
    throw new DirectFolderSecurityError(
      'output_collision',
      `Destination appeared during processing; ${partialName} was preserved.`,
    );
  }

  await adapter.publishNoReplace(partialName, request.finalName);
  const final = await adapter.snapshot(request.finalName);
  if (
    final.identity !== partial.identity ||
    final.size !== request.expectedSize ||
    final.sha256 !== request.expectedSha256
  ) {
    throw new DirectFolderSecurityError(
      'final_identity_mismatch',
      'Published output identity, size, or checksum changed after finalization.',
    );
  }
  return { finalName: request.finalName, size: final.size, sha256: final.sha256 };
}

function partialFilename(taskId: string): string {
  const safeTaskId = taskId.toLocaleLowerCase('en-US').replace(/[^a-z0-9-]/gu, '').slice(0, 64);
  if (safeTaskId.length < 8) {
    throw new DirectFolderSecurityError('write_failed', 'Task ID is not suitable for staging.');
  }
  return `cakesplitter-${safeTaskId}.partial`;
}

function constructorPrototype(value: unknown): Record<string, unknown> | undefined {
  if (typeof value !== 'function') {
    return undefined;
  }
  const prototype = (value as { prototype?: unknown }).prototype;
  return isRecord(prototype) ? prototype : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}
