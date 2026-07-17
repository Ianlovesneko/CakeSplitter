import {
  MAX_BROWSER_FALLBACK_BYTES,
  MAX_BROWSER_FALLBACK_DOWNLOADS,
  MAX_BROWSER_SELECTED_FILES,
  MAX_SAFE_INTEGER,
  MAX_SLICE_COUNT,
  expectedSliceCount,
  parseManifest,
  validateManifest,
  validatePortableFilename,
  validateSha256,
  type CakeManifest,
} from '@cakesplitter/shared-types';

export type Workspace = 'split' | 'merge' | 'inspect' | 'tasks' | 'about';
export type WorkerOperation = 'split' | 'merge' | 'inspect';
export type OutputMode = 'direct' | 'fallback' | 'read-only';
export type TaskStatus =
  | 'planned'
  | 'running'
  | 'paused'
  | 'interrupted'
  | 'permission-required'
  | 'incomplete'
  | 'failed'
  | 'completed'
  | 'cancelled';

interface MessageIdentity {
  requestId: string;
  taskId: string;
}

export type WorkerRequest =
  | (MessageIdentity & {
      type: 'start';
      operation: 'split';
      file: File;
      sliceSize: number;
      outputMode: 'direct' | 'fallback';
      directory?: FileSystemDirectoryHandle;
    })
  | (MessageIdentity & {
      type: 'start';
      operation: 'inspect';
      manifestText: string;
      files: File[];
    })
  | (MessageIdentity & {
      type: 'start';
      operation: 'merge';
      manifestText: string;
      files: File[];
      outputMode: 'direct' | 'fallback';
      directory?: FileSystemDirectoryHandle;
    })
  | (MessageIdentity & {
      type: 'control';
      command: 'pause' | 'resume' | 'cancel';
    });

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
  | (MessageIdentity & {
      type: 'progress';
      operation: WorkerOperation;
      status: 'running' | 'paused';
      bytesProcessed: number;
      totalBytes: number;
      currentSlice: number;
      sliceCount: number;
      speedBytesPerSecond: number;
      message: string;
    })
  | (MessageIdentity & {
      type: 'state';
      operation: WorkerOperation;
      status: TaskStatus;
      message: string;
    })
  | (MessageIdentity & {
      type: 'download';
      operation: 'split' | 'merge';
      filename: string;
      blob: Blob;
    })
  | (MessageIdentity & {
      type: 'result';
      operation: WorkerOperation;
      status: 'completed' | 'incomplete';
      mode: OutputMode;
      message: string;
      manifest?: CakeManifest;
      inspection?: InspectionResult;
      outputFilename?: string;
      outputSha256?: string;
    })
  | (MessageIdentity & {
      type: 'error';
      operation: WorkerOperation;
      status: 'cancelled' | 'failed' | 'permission-required' | 'incomplete';
      code: string;
      message: string;
    });

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
    throw new WorkerProtocolError('Worker command type must be a string');
  }
  assertMessageIdentity(request);
  if (request.type === 'control') {
    assertExactKeys(request, ['type', 'requestId', 'taskId', 'command']);
    if (!['pause', 'resume', 'cancel'].includes(String(request.command))) {
      throw new WorkerProtocolError('Worker control command is invalid');
    }
    return request as unknown as WorkerRequest;
  }
  if (request.type !== 'start') {
    throw new WorkerProtocolError(`Unsupported Worker command: ${request.type}`);
  }
  assertOperation(request.operation);
  switch (request.operation) {
    case 'split': {
      assertExactKeys(request, [
        'type',
        'requestId',
        'taskId',
        'operation',
        'file',
        'sliceSize',
        'outputMode',
        'directory',
      ]);
      assertFile(request.file, 'Split file');
      assertSafeInteger(request.sliceSize, 'Slice size', 1);
      const outputMode = assertWriteMode(request.outputMode);
      const directory = optionalDirectoryHandle(request.directory);
      assertModeDirectory(outputMode, directory);
      if (outputMode === 'fallback') {
        if (request.file.size > MAX_BROWSER_FALLBACK_BYTES) {
          throw new WorkerProtocolError(
            `Compatibility Split is limited to ${MAX_BROWSER_FALLBACK_BYTES} bytes`,
          );
        }
        if (expectedSliceCount(request.file.size, request.sliceSize) > MAX_BROWSER_FALLBACK_DOWNLOADS) {
          throw new WorkerProtocolError(
            `Compatibility Split is limited to ${MAX_BROWSER_FALLBACK_DOWNLOADS} downloads`,
          );
        }
      }
      return {
        type: 'start',
        requestId: request.requestId,
        taskId: request.taskId,
        operation: 'split',
        file: request.file,
        sliceSize: request.sliceSize,
        outputMode,
        ...(directory ? { directory } : {}),
      };
    }
    case 'inspect': {
      assertExactKeys(request, [
        'type',
        'requestId',
        'taskId',
        'operation',
        'manifestText',
        'files',
      ]);
      assertManifestText(request.manifestText);
      return {
        type: 'start',
        requestId: request.requestId,
        taskId: request.taskId,
        operation: 'inspect',
        manifestText: request.manifestText as string,
        files: assertFileArray(request.files),
      };
    }
    case 'merge': {
      assertExactKeys(request, [
        'type',
        'requestId',
        'taskId',
        'operation',
        'manifestText',
        'files',
        'outputMode',
        'directory',
      ]);
      const manifest = assertManifestText(request.manifestText);
      const files = assertFileArray(request.files);
      const outputMode = assertWriteMode(request.outputMode);
      const directory = optionalDirectoryHandle(request.directory);
      assertModeDirectory(outputMode, directory);
      if (outputMode === 'fallback' && manifest.original.size > MAX_BROWSER_FALLBACK_BYTES) {
        throw new WorkerProtocolError(
          `Compatibility Merge is limited to ${MAX_BROWSER_FALLBACK_BYTES} bytes`,
        );
      }
      return {
        type: 'start',
        requestId: request.requestId,
        taskId: request.taskId,
        operation: 'merge',
        manifestText: request.manifestText as string,
        files,
        outputMode,
        ...(directory ? { directory } : {}),
      };
    }
  }
}

export function parseWorkerResponse(value: unknown): WorkerResponse {
  const response = assertRecord(value, 'Worker response');
  if (typeof response.type !== 'string') {
    throw new WorkerProtocolError('Worker response type must be a string');
  }
  assertMessageIdentity(response);
  switch (response.type) {
    case 'progress':
      assertExactKeys(response, [
        'type',
        'requestId',
        'taskId',
        'operation',
        'status',
        'bytesProcessed',
        'totalBytes',
        'currentSlice',
        'sliceCount',
        'speedBytesPerSecond',
        'message',
      ]);
      assertOperation(response.operation);
      if (response.status !== 'running' && response.status !== 'paused') {
        throw new WorkerProtocolError('Worker progress status is invalid');
      }
      assertSafeInteger(response.bytesProcessed, 'Processed bytes', 0);
      assertSafeInteger(response.totalBytes, 'Total bytes', 0);
      assertSafeInteger(response.currentSlice, 'Current Slice', 0);
      assertSafeInteger(response.sliceCount, 'Slice count', 0, MAX_SLICE_COUNT);
      assertSafeInteger(response.speedBytesPerSecond, 'Processing speed', 0);
      assertShortText(response.message, 'Progress message');
      if (
        response.bytesProcessed > response.totalBytes ||
        response.currentSlice > response.sliceCount
      ) {
        throw new WorkerProtocolError('Worker progress exceeds its declared bounds');
      }
      return response as unknown as WorkerResponse;
    case 'state':
      assertExactKeys(response, [
        'type',
        'requestId',
        'taskId',
        'operation',
        'status',
        'message',
      ]);
      assertOperation(response.operation);
      assertTaskStatus(response.status);
      assertShortText(response.message, 'Task state message');
      return response as unknown as WorkerResponse;
    case 'download':
      assertExactKeys(response, [
        'type',
        'requestId',
        'taskId',
        'operation',
        'filename',
        'blob',
      ]);
      if (response.operation !== 'split' && response.operation !== 'merge') {
        throw new WorkerProtocolError('Download operation is invalid');
      }
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
        'requestId',
        'taskId',
        'operation',
        'status',
        'mode',
        'message',
        'manifest',
        'inspection',
        'outputFilename',
        'outputSha256',
      ]);
      assertOperation(response.operation);
      if (response.status !== 'completed' && response.status !== 'incomplete') {
        throw new WorkerProtocolError('Worker result status is invalid');
      }
      assertOutputMode(response.mode);
      assertShortText(response.message, 'Result message');
      if (response.manifest !== undefined) validateManifest(response.manifest);
      if (response.inspection !== undefined) validateInspection(response.inspection);
      if (response.outputFilename !== undefined) {
        if (typeof response.outputFilename !== 'string') {
          throw new WorkerProtocolError('Output filename must be a string');
        }
        validatePortableFilename(response.outputFilename);
      }
      if (response.outputSha256 !== undefined) validateSha256(response.outputSha256, 'Worker output');
      assertResultShape(response);
      return response as unknown as WorkerResponse;
    case 'error':
      assertExactKeys(response, [
        'type',
        'requestId',
        'taskId',
        'operation',
        'status',
        'code',
        'message',
      ]);
      assertOperation(response.operation);
      if (!['cancelled', 'failed', 'permission-required', 'incomplete'].includes(String(response.status))) {
        throw new WorkerProtocolError('Worker error status is invalid');
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
  const missing = assertStringArray(inspection.missing, 'missing');
  const corrupted = assertStringArray(inspection.corrupted, 'corrupted');
  const duplicates = assertStringArray(inspection.duplicates, 'duplicates');
  const unexpected = assertStringArray(inspection.unexpected, 'unexpected');
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
    if (!manifestSlice || slice.index !== manifestSlice.index || slice.filename !== manifestSlice.filename) {
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
    sliceCandidates.every((slice) => assertRecord(slice, 'Inspection Slice').state === 'verified');
  if (inspection.verified !== shouldBeVerified) {
    throw new WorkerProtocolError('Inspection verified state contradicts its evidence');
  }
}

function assertResultShape(response: Record<string, unknown>): void {
  if (response.operation === 'inspect') {
    const inspection = response.inspection as InspectionResult | undefined;
    if (
      response.mode !== 'read-only' ||
      inspection === undefined ||
      response.manifest !== undefined ||
      response.outputFilename !== undefined ||
      response.outputSha256 !== undefined ||
      response.status !== (inspection.verified ? 'completed' : 'incomplete')
    ) {
      throw new WorkerProtocolError('Inspect result fields are inconsistent');
    }
    return;
  }
  if (response.operation === 'split') {
    if (
      (response.mode !== 'fallback' && response.mode !== 'direct') ||
      response.status !== 'completed' ||
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
    (response.mode !== 'fallback' && response.mode !== 'direct') ||
    response.status !== 'completed' ||
    response.outputFilename === undefined ||
    response.outputSha256 === undefined ||
    response.manifest !== undefined ||
    response.inspection !== undefined
  ) {
    throw new WorkerProtocolError('Merge result fields are inconsistent');
  }
}

function assertManifestText(value: unknown): CakeManifest {
  if (typeof value !== 'string') {
    throw new WorkerProtocolError('Manifest text must be a string');
  }
  return parseManifest(value);
}

function assertFile(value: unknown, label: string): asserts value is File {
  if (typeof File === 'undefined' || !(value instanceof File)) {
    throw new WorkerProtocolError(`${label} must be a File`);
  }
  if (!Number.isSafeInteger(value.size) || value.size < 0 || value.size > MAX_SAFE_INTEGER) {
    throw new WorkerProtocolError(`${label} size is outside the supported integer range`);
  }
  if (!Number.isSafeInteger(value.lastModified) || value.lastModified < 0) {
    throw new WorkerProtocolError(`${label} modification time is invalid`);
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
  return (value as unknown[]).map((candidate, position) => {
    assertFile(candidate, `Selected file ${position + 1}`);
    return candidate;
  });
}

function optionalDirectoryHandle(value: unknown): FileSystemDirectoryHandle | undefined {
  if (value === undefined) return undefined;
  const handle = assertRecord(value, 'Directory handle');
  if (
    handle.kind !== 'directory' ||
    typeof handle.name !== 'string' ||
    typeof handle.getFileHandle !== 'function' ||
    typeof handle.removeEntry !== 'function' ||
    typeof handle.isSameEntry !== 'function'
  ) {
    throw new WorkerProtocolError('Directory handle is not a valid FileSystemDirectoryHandle');
  }
  validatePortableFilename(handle.name);
  return value as FileSystemDirectoryHandle;
}

function assertModeDirectory(
  mode: 'direct' | 'fallback',
  directory: FileSystemDirectoryHandle | undefined,
): void {
  if (mode === 'direct' && !directory) {
    throw new WorkerProtocolError('Direct Folder Mode requires an authorized directory handle');
  }
  if (mode === 'fallback' && directory) {
    throw new WorkerProtocolError('Compatibility Mode must not receive a directory handle');
  }
}

function assertWriteMode(value: unknown): 'direct' | 'fallback' {
  if (value !== 'direct' && value !== 'fallback') {
    throw new WorkerProtocolError('Worker output mode is invalid');
  }
  return value;
}

function assertOutputMode(value: unknown): asserts value is OutputMode {
  if (value !== 'direct' && value !== 'fallback' && value !== 'read-only') {
    throw new WorkerProtocolError('Worker result mode is invalid');
  }
}

function assertOperation(value: unknown): asserts value is WorkerOperation {
  if (value !== 'split' && value !== 'merge' && value !== 'inspect') {
    throw new WorkerProtocolError('Worker operation is invalid');
  }
}

function assertTaskStatus(value: unknown): asserts value is TaskStatus {
  if (![
    'planned',
    'running',
    'paused',
    'interrupted',
    'permission-required',
    'incomplete',
    'failed',
    'completed',
    'cancelled',
  ].includes(String(value))) {
    throw new WorkerProtocolError('Task status is invalid');
  }
}

function assertMessageIdentity(
  value: Record<string, unknown>,
): asserts value is Record<string, unknown> & MessageIdentity {
  assertIdentifier(value.requestId, 'Request ID');
  assertIdentifier(value.taskId, 'Task ID');
}

function assertIdentifier(value: unknown, label: string): asserts value is string {
  if (typeof value !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/u.test(value)) {
    throw new WorkerProtocolError(`${label} is invalid`);
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

function assertStringArray(value: unknown, label: string): string[] {
  if (!Array.isArray(value) || value.length > MAX_BROWSER_SELECTED_FILES) {
    throw new WorkerProtocolError(`${label} must be a bounded string array`);
  }
  const unique = new Set<string>();
  const output: string[] = [];
  for (const item of value) {
    if (typeof item !== 'string') {
      throw new WorkerProtocolError(`${label} must contain only strings`);
    }
    validatePortableFilename(item);
    if (unique.has(item)) {
      throw new WorkerProtocolError(`${label} must not contain duplicate filenames`);
    }
    unique.add(item);
    output.push(item);
  }
  return output;
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
  const missing = allowed.filter((key) => !Object.prototype.hasOwnProperty.call(value, key));
  const optional = new Set(['directory', 'manifest', 'inspection', 'outputFilename', 'outputSha256']);
  const requiredMissing = missing.filter((key) => !optional.has(key));
  if (unexpected.length > 0 || requiredMissing.length > 0) {
    const details = [
      unexpected.length ? `unexpected: ${unexpected.join(', ')}` : '',
      requiredMissing.length ? `missing: ${requiredMissing.join(', ')}` : '',
    ].filter(Boolean);
    throw new WorkerProtocolError(`Worker message fields are invalid (${details.join('; ')})`);
  }
}
