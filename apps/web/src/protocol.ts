import {
  MAX_BROWSER_FALLBACK_BYTES,
  MAX_BROWSER_SELECTED_FILES,
  MAX_SAFE_INTEGER,
  MAX_SLICE_COUNT,
  parseManifest,
  validateManifest,
  validatePortableFilename,
  validateSha256,
  type CakeManifest,
} from '@cakesplitter/shared-types';

export type Workspace = 'split' | 'merge' | 'inspect';

export type WorkerRequest =
  | {
      type: 'split';
      file: File;
      sliceSize: number;
      directory?: FileSystemDirectoryHandle;
    }
  | {
      type: 'inspect';
      manifestText: string;
      files: File[];
    }
  | {
      type: 'merge';
      manifestText: string;
      files: File[];
      directory?: FileSystemDirectoryHandle;
    }
  | { type: 'cancel' };

export interface SliceVerification {
  index: number;
  filename: string;
  state: 'verified' | 'missing' | 'corrupted' | 'duplicate';
  detail: string;
}

export interface InspectionResult {
  manifest: CakeManifest;
  foundSliceCount: number;
  missing: string[];
  corrupted: string[];
  duplicates: string[];
  unexpected: string[];
  verified: boolean;
  slices: SliceVerification[];
}

export type WorkerResponse =
  | {
      type: 'progress';
      operation: Workspace;
      bytesProcessed: number;
      totalBytes: number;
      currentSlice: number;
      sliceCount: number;
      message: string;
    }
  | { type: 'download'; filename: string; blob: Blob }
  | {
      type: 'result';
      operation: Workspace;
      mode: 'direct' | 'fallback' | 'read-only';
      message: string;
      manifest?: CakeManifest;
      inspection?: InspectionResult;
      outputFilename?: string;
      outputSha256?: string;
    }
  | {
      type: 'error';
      state: 'cancelled' | 'failed';
      code: string;
      message: string;
    };

export class WorkerProtocolError extends Error {
  readonly code = 'invalid_worker_message';

  constructor(message: string) {
    super(message);
    this.name = 'WorkerProtocolError';
  }
}

export function parseWorkerRequest(value: unknown): WorkerRequest {
  const request = assertRecord(value, 'Worker request');
  if (typeof request.type !== 'string') {
    throw new WorkerProtocolError('Worker command must be a string');
  }
  switch (request.type) {
    case 'cancel':
      assertExactKeys(request, ['type']);
      return { type: 'cancel' };
    case 'split': {
      assertExactKeys(request, ['type', 'file', 'sliceSize', 'directory']);
      assertFile(request.file, 'Split file');
      assertSafeInteger(request.sliceSize, 'Slice size', 1);
      const directory = optionalDirectoryHandle(request.directory);
      return {
        type: 'split',
        file: request.file,
        sliceSize: request.sliceSize,
        ...(directory ? { directory } : {}),
      };
    }
    case 'inspect':
    case 'merge': {
      assertExactKeys(
        request,
        request.type === 'merge'
          ? ['type', 'manifestText', 'files', 'directory']
          : ['type', 'manifestText', 'files'],
      );
      if (typeof request.manifestText !== 'string') {
        throw new WorkerProtocolError('Manifest text must be a string');
      }
      const manifest = parseManifest(request.manifestText);
      const files = assertFileArray(request.files);
      if (request.type === 'merge') {
        const directory = optionalDirectoryHandle(request.directory);
        if (!directory && manifest.original.size > MAX_BROWSER_FALLBACK_BYTES) {
          throw new WorkerProtocolError(
            `Compatibility Merge is limited to ${MAX_BROWSER_FALLBACK_BYTES} bytes`,
          );
        }
        return {
          type: 'merge',
          manifestText: request.manifestText,
          files,
          ...(directory ? { directory } : {}),
        };
      }
      return { type: 'inspect', manifestText: request.manifestText, files };
    }
    default:
      throw new WorkerProtocolError(`Unsupported Worker command: ${request.type}`);
  }
}

export function parseWorkerResponse(value: unknown): WorkerResponse {
  const response = assertRecord(value, 'Worker response');
  if (typeof response.type !== 'string') {
    throw new WorkerProtocolError('Worker response type must be a string');
  }
  switch (response.type) {
    case 'progress':
      assertExactKeys(response, [
        'type',
        'operation',
        'bytesProcessed',
        'totalBytes',
        'currentSlice',
        'sliceCount',
        'message',
      ]);
      assertWorkspace(response.operation);
      assertSafeInteger(response.bytesProcessed, 'Processed bytes', 0);
      assertSafeInteger(response.totalBytes, 'Total bytes', 0);
      assertSafeInteger(response.currentSlice, 'Current Slice', 0);
      assertSafeInteger(response.sliceCount, 'Slice count', 0, MAX_SLICE_COUNT);
      assertShortText(response.message, 'Progress message');
      if (
        response.bytesProcessed > response.totalBytes ||
        response.currentSlice > response.sliceCount
      ) {
        throw new WorkerProtocolError('Worker progress exceeds its declared bounds');
      }
      return response as unknown as WorkerResponse;
    case 'download':
      assertExactKeys(response, ['type', 'filename', 'blob']);
      if (typeof response.filename !== 'string') {
        throw new WorkerProtocolError('Download filename must be a string');
      }
      validatePortableFilename(response.filename);
      if (!(response.blob instanceof Blob) || response.blob.size > MAX_BROWSER_FALLBACK_BYTES) {
        throw new WorkerProtocolError('Download Blob is invalid or exceeds the fallback limit');
      }
      return response as unknown as WorkerResponse;
    case 'result':
      assertExactKeys(response, [
        'type',
        'operation',
        'mode',
        'message',
        'manifest',
        'inspection',
        'outputFilename',
        'outputSha256',
      ]);
      assertWorkspace(response.operation);
      if (!['direct', 'fallback', 'read-only'].includes(String(response.mode))) {
        throw new WorkerProtocolError('Worker result mode is invalid');
      }
      if (response.mode === 'direct') {
        throw new WorkerProtocolError('Direct-folder Worker results are disabled in this release');
      }
      assertShortText(response.message, 'Result message');
      if (response.manifest !== undefined) {
        validateManifest(response.manifest);
      }
      if (response.inspection !== undefined) {
        validateInspection(response.inspection);
      }
      if (response.outputFilename !== undefined) {
        if (typeof response.outputFilename !== 'string') {
          throw new WorkerProtocolError('Output filename must be a string');
        }
        validatePortableFilename(response.outputFilename);
      }
      if (response.outputSha256 !== undefined) {
        validateSha256(response.outputSha256, 'Worker output');
      }
      assertResultShape(response);
      return response as unknown as WorkerResponse;
    case 'error':
      assertExactKeys(response, ['type', 'state', 'code', 'message']);
      if (response.state !== 'cancelled' && response.state !== 'failed') {
        throw new WorkerProtocolError('Worker error state is invalid');
      }
      assertShortText(response.code, 'Worker error code');
      assertShortText(response.message, 'Worker error message');
      return response as unknown as WorkerResponse;
    default:
      throw new WorkerProtocolError(`Unsupported Worker response: ${response.type}`);
  }
}

function validateInspection(value: unknown): asserts value is InspectionResult {
  const inspection = assertRecord(value, 'Inspection result');
  assertExactKeys(inspection, [
    'manifest',
    'foundSliceCount',
    'missing',
    'corrupted',
    'duplicates',
    'unexpected',
    'verified',
    'slices',
  ]);
  const manifest = validateManifest(inspection.manifest);
  assertSafeInteger(inspection.foundSliceCount, 'Found Slice count', 0, MAX_BROWSER_SELECTED_FILES);
  const missing = inspection.missing;
  const corrupted = inspection.corrupted;
  const duplicates = inspection.duplicates;
  const unexpected = inspection.unexpected;
  assertStringArray(missing, 'missing');
  assertStringArray(corrupted, 'corrupted');
  assertStringArray(duplicates, 'duplicates');
  assertStringArray(unexpected, 'unexpected');
  if (typeof inspection.verified !== 'boolean') {
    throw new WorkerProtocolError('Inspection verified state must be a boolean');
  }
  if (!Array.isArray(inspection.slices) || inspection.slices.length > MAX_SLICE_COUNT) {
    throw new WorkerProtocolError('Inspection Slice ledger is invalid or too large');
  }
  const sliceCandidates = inspection.slices as unknown[];
  if (sliceCandidates.length !== manifest.sliceCount) {
    throw new WorkerProtocolError('Inspection Slice ledger length does not match the manifest');
  }
  for (const [position, candidate] of sliceCandidates.entries()) {
    const slice = assertRecord(candidate, `Inspection Slice ${position + 1}`);
    assertExactKeys(slice, ['index', 'filename', 'state', 'detail']);
    assertSafeInteger(slice.index, 'Inspection Slice index', 1, MAX_SLICE_COUNT);
    if (typeof slice.filename !== 'string') {
      throw new WorkerProtocolError('Inspection Slice filename must be a string');
    }
    validatePortableFilename(slice.filename);
    const manifestSlice = manifest.slices[position];
    if (
      !manifestSlice ||
      slice.index !== manifestSlice.index ||
      slice.filename !== manifestSlice.filename
    ) {
      throw new WorkerProtocolError('Inspection Slice ledger does not match the manifest');
    }
    if (!['verified', 'missing', 'corrupted', 'duplicate'].includes(String(slice.state))) {
      throw new WorkerProtocolError('Inspection Slice state is invalid');
    }
    assertShortText(slice.detail, 'Inspection Slice detail');
  }
  const shouldBeVerified =
    missing.length === 0 &&
    corrupted.length === 0 &&
    duplicates.length === 0 &&
    unexpected.length === 0 &&
    sliceCandidates.every((slice) =>
      assertRecord(slice, 'Inspection Slice').state === 'verified',
    );
  if (inspection.verified !== shouldBeVerified) {
    throw new WorkerProtocolError('Inspection verified state contradicts its evidence');
  }
}

function assertResultShape(response: Record<string, unknown>): void {
  if (response.operation === 'inspect') {
    if (
      response.mode !== 'read-only' ||
      response.inspection === undefined ||
      response.manifest !== undefined ||
      response.outputFilename !== undefined ||
      response.outputSha256 !== undefined
    ) {
      throw new WorkerProtocolError('Inspect result fields are inconsistent');
    }
    return;
  }
  if (response.operation === 'split') {
    if (
      response.mode !== 'fallback' ||
      response.manifest === undefined ||
      response.inspection !== undefined ||
      response.outputFilename !== undefined ||
      response.outputSha256 !== undefined
    ) {
      throw new WorkerProtocolError('Split result fields are inconsistent');
    }
    return;
  }
  if (
    response.mode !== 'fallback' ||
    response.outputFilename === undefined ||
    response.outputSha256 === undefined ||
    response.manifest !== undefined ||
    response.inspection !== undefined
  ) {
    throw new WorkerProtocolError('Merge result fields are inconsistent');
  }
}

function assertFile(value: unknown, label: string): asserts value is File {
  if (typeof File === 'undefined' || !(value instanceof File)) {
    throw new WorkerProtocolError(`${label} must be a File`);
  }
  if (!Number.isSafeInteger(value.size) || value.size < 0 || value.size > MAX_SAFE_INTEGER) {
    throw new WorkerProtocolError(`${label} size is outside the supported integer range`);
  }
  validatePortableFilename(value.name);
}

function assertFileArray(value: unknown): File[] {
  if (!Array.isArray(value)) {
    throw new WorkerProtocolError('Selected files must be an array');
  }
  if (value.length > MAX_BROWSER_SELECTED_FILES) {
    throw new WorkerProtocolError(
      `Selected file count exceeds the browser maximum of ${MAX_BROWSER_SELECTED_FILES}`,
    );
  }
  const files: File[] = [];
  for (const [position, candidate] of (value as unknown[]).entries()) {
    assertFile(candidate, `Selected file ${position + 1}`);
    files.push(candidate);
  }
  return files;
}

function optionalDirectoryHandle(value: unknown): FileSystemDirectoryHandle | undefined {
  if (value === undefined) {
    return undefined;
  }
  const handle = assertRecord(value, 'Directory handle');
  if (
    handle.kind !== 'directory' ||
    typeof handle.name !== 'string' ||
    typeof handle.getFileHandle !== 'function' ||
    typeof handle.removeEntry !== 'function'
  ) {
    throw new WorkerProtocolError('Directory handle is not a valid FileSystemDirectoryHandle');
  }
  return value as FileSystemDirectoryHandle;
}

function assertWorkspace(value: unknown): asserts value is Workspace {
  if (value !== 'split' && value !== 'merge' && value !== 'inspect') {
    throw new WorkerProtocolError('Worker operation is invalid');
  }
}

function assertSafeInteger(
  value: unknown,
  label: string,
  minimum: number,
  maximum = MAX_SAFE_INTEGER,
): asserts value is number {
  if (!Number.isSafeInteger(value) || (value as number) < minimum || (value as number) > maximum) {
    throw new WorkerProtocolError(`${label} is outside the supported integer range`);
  }
}

function assertStringArray(value: unknown, label: string): asserts value is string[] {
  if (!Array.isArray(value) || value.length > MAX_BROWSER_SELECTED_FILES) {
    throw new WorkerProtocolError(`${label} must be a bounded string array`);
  }
  const unique = new Set<string>();
  for (const item of value) {
    if (typeof item !== 'string') {
      throw new WorkerProtocolError(`${label} must contain only strings`);
    }
    validatePortableFilename(item);
    if (unique.has(item)) {
      throw new WorkerProtocolError(`${label} must not contain duplicate filenames`);
    }
    unique.add(item);
  }
}

function assertShortText(value: unknown, label: string): asserts value is string {
  if (typeof value !== 'string' || value.length > 4_096) {
    throw new WorkerProtocolError(`${label} must be a bounded string`);
  }
}

function assertRecord(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new WorkerProtocolError(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function assertExactKeys(value: Record<string, unknown>, allowed: string[]): void {
  const unexpected = Object.keys(value).filter((key) => !allowed.includes(key));
  if (unexpected.length > 0) {
    throw new WorkerProtocolError(`Worker message has unexpected fields: ${unexpected.join(', ')}`);
  }
}
