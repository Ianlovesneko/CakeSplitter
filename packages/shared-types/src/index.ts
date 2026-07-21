export const FORMAT_IDENTIFIER = 'cakesplitter' as const;
export const FORMAT_VERSION = '1.0' as const;
export const MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER;
export const MAX_MANIFEST_BYTES = 16 * 1024 * 1024;
export const MAX_SLICE_COUNT = 50_000;
export const MAX_FILENAME_BYTES = 200;
export const MAX_JSON_NESTING = 16;
export const MAX_BROWSER_SELECTED_FILES = 10_000;
export const MAX_BROWSER_FALLBACK_BYTES = 256 * 1024 * 1024;
export const MAX_BROWSER_FALLBACK_DOWNLOADS = 1_000;
export const CLI_SCHEMA_VERSION = 1 as const;

export type CliCommand =
  | 'split'
  | 'merge'
  | 'inspect'
  | 'verify'
  | 'plan'
  | 'version'
  | 'help'
  | 'unknown';

export type CliErrorCategory =
  | 'usage'
  | 'source'
  | 'destination'
  | 'package'
  | 'integrity'
  | 'permission'
  | 'storage'
  | 'conflict'
  | 'recovery'
  | 'capacity'
  | 'cancellation'
  | 'internal';

export interface CliStructuredError {
  code: string;
  category: CliErrorCategory;
  message: string;
  technicalMessage: string;
  retryable: boolean;
  suggestedAction: string;
  operationId?: string;
}

export interface CliFinalResult {
  schemaVersion: typeof CLI_SCHEMA_VERSION;
  applicationVersion: string;
  command: CliCommand;
  status: 'completed' | 'failed' | 'cancelled';
  result: unknown;
  warnings: string[];
  error: CliStructuredError | null;
  startedAt: string;
  completedAt: string;
  durationMs: number;
}

export type CliJsonlEventName =
  | 'started'
  | 'preflight'
  | 'progress'
  | 'warning'
  | 'paused'
  | 'resumed'
  | 'completed'
  | 'failed'
  | 'cancelled';

export interface CliJsonlEvent {
  schemaVersion: typeof CLI_SCHEMA_VERSION;
  event: CliJsonlEventName;
  command: CliCommand;
  operationId: string;
  timestamp: string;
  sequence: number;
  payload: Record<string, unknown>;
}

export class CliContractValidationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CliContractValidationError';
  }
}

export function validateCliFinalResult(value: unknown): CliFinalResult {
  const record = cliRecord(value, 'CLI final result');
  cliExactKeys(record, [
    'schemaVersion',
    'applicationVersion',
    'command',
    'status',
    'result',
    'warnings',
    'error',
    'startedAt',
    'completedAt',
    'durationMs',
  ]);
  if (record.schemaVersion !== CLI_SCHEMA_VERSION) cliFailure('unsupported CLI schemaVersion');
  if (typeof record.applicationVersion !== 'string' || record.applicationVersion.length === 0) {
    cliFailure('applicationVersion must be a non-empty string');
  }
  const command = cliCommand(record.command);
  if (!['completed', 'failed', 'cancelled'].includes(String(record.status))) {
    cliFailure('status is invalid');
  }
  if (!Array.isArray(record.warnings) || record.warnings.length > 100 ||
      record.warnings.some((warning) => typeof warning !== 'string' || warning.length > 2000)) {
    cliFailure('warnings must be a string array');
  }
  const error = record.error === null ? null : validateCliStructuredError(record.error);
  if (record.status === 'completed' && error !== null) cliFailure('completed result cannot contain an error');
  if (record.status !== 'completed' && error === null) cliFailure('terminal failure requires an error');
  cliTimestamp(record.startedAt, 'startedAt');
  cliTimestamp(record.completedAt, 'completedAt');
  cliNonNegativeInteger(record.durationMs, 'durationMs');
  return {
    schemaVersion: CLI_SCHEMA_VERSION,
    applicationVersion: record.applicationVersion,
    command,
    status: record.status as CliFinalResult['status'],
    result: record.result,
    warnings: record.warnings as string[],
    error,
    startedAt: record.startedAt as string,
    completedAt: record.completedAt as string,
    durationMs: record.durationMs as number,
  };
}

export function validateCliJsonlEvent(value: unknown): CliJsonlEvent {
  const record = cliRecord(value, 'CLI JSONL event');
  cliExactKeys(record, [
    'schemaVersion',
    'event',
    'command',
    'operationId',
    'timestamp',
    'sequence',
    'payload',
  ]);
  if (record.schemaVersion !== CLI_SCHEMA_VERSION) cliFailure('unsupported CLI schemaVersion');
  const events: CliJsonlEventName[] = [
    'started', 'preflight', 'progress', 'warning', 'paused', 'resumed',
    'completed', 'failed', 'cancelled',
  ];
  if (!events.includes(record.event as CliJsonlEventName)) cliFailure('event is invalid');
  if (typeof record.operationId !== 'string' || !UUID_PATTERN.test(record.operationId)) {
    cliFailure('operationId must be a UUID');
  }
  cliTimestamp(record.timestamp, 'timestamp');
  cliPositiveInteger(record.sequence, 'sequence');
  const payload = cliRecord(record.payload, 'payload');
  return {
    schemaVersion: CLI_SCHEMA_VERSION,
    event: record.event as CliJsonlEventName,
    command: cliCommand(record.command),
    operationId: record.operationId,
    timestamp: record.timestamp as string,
    sequence: record.sequence as number,
    payload,
  };
}

export function validateCliJsonlStream(values: unknown[]): CliJsonlEvent[] {
  const events = values.map(validateCliJsonlEvent);
  if (events.length === 0) cliFailure('JSONL stream must contain at least one event');
  const first = events[0]!;
  const terminal = new Set<CliJsonlEventName>(['completed', 'failed', 'cancelled']);
  events.forEach((event, index) => {
    if (event.operationId !== first.operationId || event.command !== first.command) {
      cliFailure('JSONL stream operation identity changed');
    }
    if (event.sequence !== index + 1) cliFailure('JSONL sequence must be contiguous and monotonic');
  });
  const terminals = events.filter((event) => terminal.has(event.event));
  if (terminals.length !== 1 || terminals[0] !== events[events.length - 1]) {
    cliFailure('JSONL stream must end with exactly one terminal event');
  }
  return events;
}

export function validateCliStructuredError(value: unknown): CliStructuredError {
  const record = cliRecord(value, 'CLI structured error');
  const required = ['code', 'category', 'message', 'technicalMessage', 'retryable', 'suggestedAction'];
  const allowed = new Set([...required, 'operationId']);
  if (required.some((key) => !(key in record)) || Object.keys(record).some((key) => !allowed.has(key))) {
    cliFailure('structured error fields are invalid');
  }
  const categories: CliErrorCategory[] = [
    'usage', 'source', 'destination', 'package', 'integrity', 'permission',
    'storage', 'conflict', 'recovery', 'capacity', 'cancellation', 'internal',
  ];
  if (!categories.includes(record.category as CliErrorCategory)) cliFailure('error category is invalid');
  for (const key of ['code', 'message', 'technicalMessage', 'suggestedAction'] as const) {
    const maxLength = key === 'code' ? 80 : 2000;
    if (typeof record[key] !== 'string' || record[key].length > maxLength ||
        (key !== 'technicalMessage' && record[key].length === 0)) {
      cliFailure(`${key} must be a valid string`);
    }
  }
  if (typeof record.retryable !== 'boolean') cliFailure('retryable must be boolean');
  if (record.operationId !== undefined &&
      (typeof record.operationId !== 'string' || !UUID_PATTERN.test(record.operationId))) {
    cliFailure('operationId must be a UUID when present');
  }
  return record as unknown as CliStructuredError;
}

const CLI_COMMANDS = new Set<CliCommand>([
  'split', 'merge', 'inspect', 'verify', 'plan', 'version', 'help', 'unknown',
]);
const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu;
const RFC3339_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/u;

function cliCommand(value: unknown): CliCommand {
  if (typeof value !== 'string' || !CLI_COMMANDS.has(value as CliCommand)) cliFailure('command is invalid');
  return value as CliCommand;
}

function cliRecord(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) cliFailure(`${label} must be an object`);
  return value as Record<string, unknown>;
}

function cliExactKeys(record: Record<string, unknown>, keys: string[]): void {
  const expected = new Set(keys);
  if (keys.some((key) => !(key in record)) || Object.keys(record).some((key) => !expected.has(key))) {
    cliFailure('object fields do not match the versioned CLI schema');
  }
}

function cliTimestamp(value: unknown, label: string): void {
  if (typeof value !== 'string' || !RFC3339_PATTERN.test(value) || Number.isNaN(Date.parse(value))) {
    cliFailure(`${label} must be an RFC3339 timestamp`);
  }
}

function cliPositiveInteger(value: unknown, label: string): void {
  if (!Number.isSafeInteger(value) || (value as number) < 1) cliFailure(`${label} must be a positive integer`);
}

function cliNonNegativeInteger(value: unknown, label: string): void {
  if (!Number.isSafeInteger(value) || (value as number) < 0) cliFailure(`${label} must be a non-negative integer`);
}

function cliFailure(message: string): never {
  throw new CliContractValidationError(message);
}

export interface OriginalFile {
  filename: string;
  size: number;
  sha256: string;
}

export interface SliceEntry {
  index: number;
  filename: string;
  offset: number;
  size: number;
  sha256: string;
}

export interface CakeManifest {
  format: typeof FORMAT_IDENTIFIER;
  version: typeof FORMAT_VERSION;
  packageId: string;
  createdAt: string;
  original: OriginalFile;
  targetSliceSize: number;
  sliceCount: number;
  slices: SliceEntry[];
}

export interface SlicePlanEntry {
  index: number;
  filename: string;
  offset: number;
  size: number;
}

export class ManifestValidationError extends Error {
  readonly code = 'invalid_manifest';

  constructor(message: string) {
    super(message);
    this.name = 'ManifestValidationError';
  }
}

export function expectedSliceCount(size: number, targetSliceSize: number): number {
  assertSafeInteger(size, 'original size', 0);
  assertSafeInteger(targetSliceSize, 'target slice size', 1);
  return size === 0 ? 0 : Math.ceil(size / targetSliceSize);
}

export function sliceIndexWidth(sliceCount: number): number {
  return Math.max(3, String(Math.max(1, sliceCount)).length);
}

export function sliceFilename(
  originalFilename: string,
  index: number,
  width: number,
): string {
  return `${originalFilename}.${String(index).padStart(width, '0')}.slice`;
}

export function manifestFilename(originalFilename: string): string {
  return `${originalFilename}.cake.json`;
}

export function planSlices(
  originalFilename: string,
  size: number,
  targetSliceSize: number,
): SlicePlanEntry[] {
  validatePortableFilename(originalFilename);
  const count = expectedSliceCount(size, targetSliceSize);
  if (count > MAX_SLICE_COUNT) {
    throw new ManifestValidationError(
      `Slice count ${count} exceeds the supported maximum of ${MAX_SLICE_COUNT}`,
    );
  }
  const width = sliceIndexWidth(count);
  return Array.from({ length: count }, (_, position) => {
    const index = position + 1;
    const offset = position * targetSliceSize;
    const filename = sliceFilename(originalFilename, index, width);
    validatePortableFilename(filename);
    return {
      index,
      filename,
      offset,
      size: Math.min(targetSliceSize, size - offset),
    };
  });
}

export function parseManifest(json: string): CakeManifest {
  if (utf8ByteLength(json) > MAX_MANIFEST_BYTES) {
    throw new ManifestValidationError(
      `Manifest exceeds the ${MAX_MANIFEST_BYTES}-byte limit`,
    );
  }
  validateJsonNesting(json);
  let value: unknown;
  try {
    value = JSON.parse(json) as unknown;
  } catch (error) {
    throw new ManifestValidationError(
      `Manifest is not valid JSON: ${error instanceof Error ? error.message : 'unknown error'}`,
    );
  }
  return validateManifest(value);
}

export function validateManifest(value: unknown): CakeManifest {
  const manifest = assertRecord(value, 'manifest');
  assertExactKeys(manifest, [
    'format',
    'version',
    'packageId',
    'createdAt',
    'original',
    'targetSliceSize',
    'sliceCount',
    'slices',
  ]);
  if (manifest.format !== FORMAT_IDENTIFIER) {
    throw new ManifestValidationError(`Unsupported format identifier: ${String(manifest.format)}`);
  }
  if (manifest.version !== FORMAT_VERSION) {
    throw new ManifestValidationError(`Unsupported format version: ${String(manifest.version)}`);
  }
  if (
    typeof manifest.packageId !== 'string' ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
      manifest.packageId,
    )
  ) {
    throw new ManifestValidationError('Invalid package ID');
  }
  if (
    typeof manifest.createdAt !== 'string' ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(
      manifest.createdAt,
    ) ||
    Number.isNaN(Date.parse(manifest.createdAt))
  ) {
    throw new ManifestValidationError('Invalid creation timestamp');
  }

  const original = assertRecord(manifest.original, 'original');
  assertExactKeys(original, ['filename', 'size', 'sha256']);
  if (typeof original.filename !== 'string') {
    throw new ManifestValidationError('Original filename must be a string');
  }
  validatePortableFilename(original.filename);
  assertSafeInteger(original.size, 'original size', 0);
  validateSha256(original.sha256, 'original file');
  assertSafeInteger(manifest.targetSliceSize, 'target slice size', 1);
  assertSafeInteger(manifest.sliceCount, 'slice count', 0);
  if (!Array.isArray(manifest.slices)) {
    throw new ManifestValidationError('slices must be an array');
  }
  const originalFilename = original.filename;
  const originalSize = original.size;
  const targetSliceSize = manifest.targetSliceSize;
  const sliceCount = manifest.sliceCount;
  if (sliceCount > MAX_SLICE_COUNT || manifest.slices.length > MAX_SLICE_COUNT) {
    throw new ManifestValidationError(
      `Slice count exceeds the supported maximum of ${MAX_SLICE_COUNT}`,
    );
  }
  const expectedCount = expectedSliceCount(originalSize, targetSliceSize);
  if (sliceCount !== expectedCount) {
    throw new ManifestValidationError('sliceCount does not match the original size and target size');
  }
  if (manifest.slices.length !== sliceCount) {
    throw new ManifestValidationError('Slice table length does not match sliceCount');
  }

  const width = sliceIndexWidth(sliceCount);
  const indexes = new Set<number>();
  const filenames = new Set<string>();
  let expectedOffset = 0;
  const slices = manifest.slices.map((candidate, position): SliceEntry => {
    const slice = assertRecord(candidate, `slice ${position + 1}`);
    assertExactKeys(slice, ['index', 'filename', 'offset', 'size', 'sha256']);
    assertSafeInteger(slice.index, 'slice index', 1);
    if (indexes.has(slice.index)) {
      throw new ManifestValidationError(`Duplicate slice index: ${slice.index}`);
    }
    indexes.add(slice.index);
    if (slice.index !== position + 1) {
      throw new ManifestValidationError(
        `Slice indexes must be ordered and contiguous; expected ${position + 1}, found ${slice.index}`,
      );
    }
    if (typeof slice.filename !== 'string') {
      throw new ManifestValidationError('Slice filename must be a string');
    }
    validatePortableFilename(slice.filename);
    if (filenames.has(slice.filename)) {
      throw new ManifestValidationError(`Duplicate slice filename: ${slice.filename}`);
    }
    filenames.add(slice.filename);
    const expectedFilename = sliceFilename(originalFilename, slice.index, width);
    if (slice.filename !== expectedFilename) {
      throw new ManifestValidationError(
        `Invalid slice filename at index ${slice.index}; expected ${expectedFilename}`,
      );
    }
    assertSafeInteger(slice.offset, 'slice offset', 0);
    if (slice.offset !== expectedOffset) {
      throw new ManifestValidationError(
        `Invalid slice offset at index ${slice.index}; expected ${expectedOffset}`,
      );
    }
    assertSafeInteger(slice.size, 'slice size', 0);
    const expectedSize = Math.min(
      targetSliceSize,
      originalSize - expectedOffset,
    );
    if (slice.size !== expectedSize) {
      throw new ManifestValidationError(
        `Invalid slice size at index ${slice.index}; expected ${expectedSize}`,
      );
    }
    validateSha256(slice.sha256, `slice ${slice.index}`);
    expectedOffset += slice.size;
    return {
      index: slice.index,
      filename: slice.filename,
      offset: slice.offset,
      size: slice.size,
      sha256: slice.sha256,
    };
  });
  if (expectedOffset !== originalSize) {
    throw new ManifestValidationError('Slice ranges do not exactly cover the original file');
  }
  return {
    format: FORMAT_IDENTIFIER,
    version: FORMAT_VERSION,
    packageId: manifest.packageId,
    createdAt: manifest.createdAt,
    original: {
      filename: originalFilename,
      size: originalSize,
      sha256: original.sha256,
    },
    targetSliceSize,
    sliceCount,
    slices,
  };
}

export function validatePortableFilename(filename: string): void {
  const byteLength = utf8ByteLength(filename);
  if (byteLength > MAX_FILENAME_BYTES) {
    throw new ManifestValidationError(
      `Portable filename is ${byteLength} UTF-8 bytes; maximum is ${MAX_FILENAME_BYTES}`,
    );
  }
  const first = [...filename][0];
  const last = [...filename].at(-1);
  if (
    filename.length === 0 ||
    filename === '.' ||
    filename === '..' ||
    /[/\\:<>"|?*]/u.test(filename) ||
    [...filename].some((character) => /\p{Cc}/u.test(character)) ||
    (first !== undefined && /\p{White_Space}/u.test(first)) ||
    last === '.' ||
    (last !== undefined && /\p{White_Space}/u.test(last)) ||
    filename.includes('../') ||
    filename.includes('..\\') ||
    isWindowsReservedName(filename)
  ) {
    throw new ManifestValidationError(`Unsafe or invalid portable filename: ${filename}`);
  }
}

export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function isWindowsReservedName(filename: string): boolean {
  const basename = (filename.split('.')[0] ?? '').toUpperCase();
  if (['CON', 'PRN', 'AUX', 'NUL', 'CLOCK$'].includes(basename)) {
    return true;
  }
  if (['COM¹', 'COM²', 'COM³', 'LPT¹', 'LPT²', 'LPT³'].includes(basename)) {
    return true;
  }
  return /^(?:COM|LPT)[1-9]$/u.test(basename);
}

function validateJsonNesting(json: string): void {
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (const character of json) {
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (character === '\\') {
        escaped = true;
      } else if (character === '"') {
        inString = false;
      }
      continue;
    }
    if (character === '"') {
      inString = true;
    } else if (character === '{' || character === '[') {
      depth += 1;
      if (depth > MAX_JSON_NESTING) {
        throw new ManifestValidationError(
          `Manifest JSON nesting exceeds the maximum depth of ${MAX_JSON_NESTING}`,
        );
      }
    } else if (character === '}' || character === ']') {
      depth = Math.max(0, depth - 1);
    }
  }
}

export function validateSha256(value: unknown, label: string): asserts value is string {
  if (typeof value !== 'string' || !/^[0-9a-f]{64}$/.test(value)) {
    throw new ManifestValidationError(`Invalid SHA-256 value for ${label}`);
  }
}

function assertSafeInteger(
  value: unknown,
  label: string,
  minimum: number,
): asserts value is number {
  if (
    typeof value !== 'number' ||
    !Number.isSafeInteger(value) ||
    value < minimum ||
    value > MAX_SAFE_INTEGER
  ) {
    throw new ManifestValidationError(`${label} is outside the supported integer range`);
  }
}

function assertRecord(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new ManifestValidationError(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function assertExactKeys(value: Record<string, unknown>, expected: string[]): void {
  const keys = Object.keys(value);
  const unexpected = keys.filter((key) => !expected.includes(key));
  const missing = expected.filter((key) => !(key in value));
  if (unexpected.length > 0 || missing.length > 0) {
    throw new ManifestValidationError(
      `Manifest fields do not match the format (missing: ${missing.join(', ') || 'none'}; unexpected: ${unexpected.join(', ') || 'none'})`,
    );
  }
}

const SHA256_CONSTANTS = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
  0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
  0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
  0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
  0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
  0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
  0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
  0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
  0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

export class IncrementalSha256 {
  private readonly state = new Uint32Array([
    0x6a09e667,
    0xbb67ae85,
    0x3c6ef372,
    0xa54ff53a,
    0x510e527f,
    0x9b05688c,
    0x1f83d9ab,
    0x5be0cd19,
  ]);
  private readonly buffer = new Uint8Array(64);
  private readonly words = new Uint32Array(64);
  private bufferLength = 0;
  private bytesHashed = 0;
  private finished = false;

  update(input: Uint8Array): this {
    if (this.finished) {
      throw new Error('SHA-256 state is already finalized');
    }
    let position = 0;
    this.bytesHashed += input.length;
    while (position < input.length) {
      const take = Math.min(64 - this.bufferLength, input.length - position);
      this.buffer.set(input.subarray(position, position + take), this.bufferLength);
      this.bufferLength += take;
      position += take;
      if (this.bufferLength === 64) {
        this.processBlock(this.buffer);
        this.bufferLength = 0;
      }
    }
    return this;
  }

  digestHex(): string {
    return Array.from(this.digestBytes(), (byte) => byte.toString(16).padStart(2, '0')).join('');
  }

  digestBytes(): Uint8Array {
    if (!this.finished) {
      const originalLength = this.bytesHashed;
      const paddingLength = this.bufferLength < 56 ? 56 - this.bufferLength : 120 - this.bufferLength;
      const padding = new Uint8Array(paddingLength + 8);
      padding[0] = 0x80;
      const high = Math.floor(originalLength / 0x20000000) >>> 0;
      const low = (originalLength * 8) >>> 0;
      const view = new DataView(padding.buffer);
      view.setUint32(padding.length - 8, high, false);
      view.setUint32(padding.length - 4, low, false);
      this.update(padding);
      this.finished = true;
    }
    const digest = new Uint8Array(32);
    const view = new DataView(digest.buffer);
    for (let index = 0; index < this.state.length; index += 1) {
      view.setUint32(index * 4, this.state[index] ?? 0, false);
    }
    return digest;
  }

  private processBlock(block: Uint8Array): void {
    const view = new DataView(block.buffer, block.byteOffset, block.byteLength);
    for (let index = 0; index < 16; index += 1) {
      this.words[index] = view.getUint32(index * 4, false);
    }
    for (let index = 16; index < 64; index += 1) {
      const w15 = this.words[index - 15] ?? 0;
      const w2 = this.words[index - 2] ?? 0;
      const s0 = rotateRight(w15, 7) ^ rotateRight(w15, 18) ^ (w15 >>> 3);
      const s1 = rotateRight(w2, 17) ^ rotateRight(w2, 19) ^ (w2 >>> 10);
      this.words[index] =
        ((this.words[index - 16] ?? 0) + s0 + (this.words[index - 7] ?? 0) + s1) >>> 0;
    }

    let a = this.state[0] ?? 0;
    let b = this.state[1] ?? 0;
    let c = this.state[2] ?? 0;
    let d = this.state[3] ?? 0;
    let e = this.state[4] ?? 0;
    let f = this.state[5] ?? 0;
    let g = this.state[6] ?? 0;
    let h = this.state[7] ?? 0;
    for (let index = 0; index < 64; index += 1) {
      const s1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25);
      const choice = (e & f) ^ (~e & g);
      const temp1 =
        (h + s1 + choice + (SHA256_CONSTANTS[index] ?? 0) + (this.words[index] ?? 0)) >>> 0;
      const s0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22);
      const majority = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (s0 + majority) >>> 0;
      h = g;
      g = f;
      f = e;
      e = (d + temp1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) >>> 0;
    }
    this.state[0] = ((this.state[0] ?? 0) + a) >>> 0;
    this.state[1] = ((this.state[1] ?? 0) + b) >>> 0;
    this.state[2] = ((this.state[2] ?? 0) + c) >>> 0;
    this.state[3] = ((this.state[3] ?? 0) + d) >>> 0;
    this.state[4] = ((this.state[4] ?? 0) + e) >>> 0;
    this.state[5] = ((this.state[5] ?? 0) + f) >>> 0;
    this.state[6] = ((this.state[6] ?? 0) + g) >>> 0;
    this.state[7] = ((this.state[7] ?? 0) + h) >>> 0;
  }
}

function rotateRight(value: number, shift: number): number {
  return (value >>> shift) | (value << (32 - shift));
}
