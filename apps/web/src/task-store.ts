import {
  MAX_SAFE_INTEGER,
  MAX_SLICE_COUNT,
  validatePortableFilename,
  validateSha256,
} from '@cakesplitter/shared-types';

import type { OutputMode, TaskStatus, WorkerOperation } from './protocol';

export const TASK_SCHEMA_VERSION = 1;
export const MAX_PERSISTED_TASKS = 200;
export const MAX_TASK_METADATA_BYTES = 256 * 1024;

export interface CapabilitySnapshot {
  directFolder: boolean;
  reason: string;
}

export interface RecoveryRequirements {
  sourceFile: boolean;
  manifest: boolean;
  slices: boolean;
  outputDirectory: boolean;
}

export interface PersistedTask {
  schemaVersion: typeof TASK_SCHEMA_VERSION;
  taskId: string;
  operation: WorkerOperation;
  packageId?: string;
  originalFilename: string;
  expectedSize: number;
  expectedSha256?: string;
  sliceSize?: number;
  sliceCount: number;
  completedSliceIndexes: number[];
  outputMode: OutputMode;
  status: TaskStatus;
  createdAt: string;
  updatedAt: string;
  capability: CapabilitySnapshot;
  recovery: RecoveryRequirements;
}

interface OpfsFileHandle {
  getFile(): Promise<File>;
  createWritable(): Promise<{
    write(data: string): Promise<void>;
    close(): Promise<void>;
    abort(reason?: unknown): Promise<void>;
  }>;
}

interface OpfsDirectoryHandle {
  getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<OpfsDirectoryHandle>;
  getFileHandle(name: string, options?: { create?: boolean }): Promise<OpfsFileHandle>;
  removeEntry(name: string, options?: { recursive?: boolean }): Promise<void>;
  values(): AsyncIterable<{ kind: 'file' | 'directory'; name: string }>;
}

export class TaskStoreUnavailableError extends Error {
  constructor(message = 'Browser-local task storage is unavailable.') {
    super(message);
    this.name = 'TaskStoreUnavailableError';
  }
}

export class TaskStore {
  private clearOperation: Promise<number> | undefined;
  private clearing = false;
  private generation = 0;
  private operationQueue: Promise<void> = Promise.resolve();

  available(): boolean {
    if (typeof navigator === 'undefined' || navigator.storage === undefined) return false;
    const storage = navigator.storage as StorageManager & { getDirectory?: () => Promise<unknown> };
    return typeof storage.getDirectory === 'function';
  }

  captureGeneration(): number {
    return this.generation;
  }

  isCurrentGeneration(generation: number): boolean {
    return generation === this.generation && !this.clearing;
  }

  list(): Promise<PersistedTask[]> {
    return this.enqueue(() => this.listUnlocked());
  }

  save(task: PersistedTask, generation = this.generation): Promise<boolean> {
    const validated = validatePersistedTask(task);
    if (!this.isCurrentGeneration(generation)) return Promise.resolve(false);
    return this.enqueue(async () => {
      if (!this.isCurrentGeneration(generation)) return false;
      await this.saveUnlocked(validated);
      if (!this.isCurrentGeneration(generation)) {
        await this.discardUnlocked(validated.taskId);
        return false;
      }
      return true;
    });
  }

  discard(taskId: string): Promise<void> {
    taskStorageFilename(taskId);
    return this.enqueue(() => this.discardUnlocked(taskId));
  }

  clear(): Promise<number> {
    if (this.clearOperation) return this.clearOperation;
    const generation = this.generation + 1;
    this.generation = generation;
    this.clearing = true;
    const queued = this.enqueue(async () => {
      await this.clearUnlocked();
      return generation;
    });
    const operation = queued.finally(() => {
      this.clearing = false;
      this.clearOperation = undefined;
    });
    this.clearOperation = operation;
    return operation;
  }

  markInterrupted(generation = this.generation): Promise<PersistedTask[]> {
    return this.enqueue(async () => {
      if (!this.isCurrentGeneration(generation)) return [];
      const tasks = await this.listUnlocked();
      const updated: PersistedTask[] = [];
      for (const task of tasks) {
        if (!this.isCurrentGeneration(generation)) return [];
        if (task.status === 'running' || task.status === 'paused') {
          const interrupted = {
            ...task,
            status: 'interrupted' as const,
            updatedAt: new Date().toISOString(),
          };
          await this.saveUnlocked(interrupted);
          updated.push(interrupted);
        } else {
          updated.push(task);
        }
      }
      return this.isCurrentGeneration(generation)
        ? updated.sort((left, right) => right.updatedAt.localeCompare(left.updatedAt))
        : [];
    });
  }

  async storageEstimate(): Promise<{ usage: number; quota: number | undefined }> {
    if (typeof navigator === 'undefined' || typeof navigator.storage?.estimate !== 'function') {
      return { usage: 0, quota: undefined };
    }
    const estimate = await navigator.storage.estimate();
    return { usage: estimate.usage ?? 0, quota: estimate.quota };
  }

  private async listUnlocked(): Promise<PersistedTask[]> {
    const directory = await tasksDirectory(false);
    if (!directory) return [];
    const tasks: PersistedTask[] = [];
    for await (const entry of directory.values()) {
      if (entry.kind !== 'file' || !entry.name.endsWith('.json')) continue;
      const file = await (await directory.getFileHandle(entry.name)).getFile();
      if (file.size > MAX_TASK_METADATA_BYTES) {
        throw new TaskStoreUnavailableError(`Task metadata file is oversized: ${entry.name}`);
      }
      tasks.push(parsePersistedTask(await file.text()));
      if (tasks.length > MAX_PERSISTED_TASKS) {
        throw new TaskStoreUnavailableError('Task metadata count exceeds the local safety limit.');
      }
    }
    return tasks.sort((left, right) => right.updatedAt.localeCompare(left.updatedAt));
  }

  private async saveUnlocked(validated: PersistedTask): Promise<void> {
    const existing = await this.listUnlocked();
    if (!existing.some((entry) => entry.taskId === validated.taskId) && existing.length >= MAX_PERSISTED_TASKS) {
      throw new TaskStoreUnavailableError(
        `Discard an old task before creating more than ${MAX_PERSISTED_TASKS} records.`,
      );
    }
    const text = `${JSON.stringify(validated, null, 2)}\n`;
    if (new TextEncoder().encode(text).byteLength > MAX_TASK_METADATA_BYTES) {
      throw new TaskStoreUnavailableError('Task metadata exceeds the local safety limit.');
    }
    const directory = await tasksDirectory(true);
    if (!directory) throw new TaskStoreUnavailableError();
    const writable = await (await directory.getFileHandle(taskStorageFilename(validated.taskId), { create: true })).createWritable();
    try {
      await writable.write(text);
      await writable.close();
    } catch (error) {
      await writable.abort(error).catch(() => undefined);
      throw error;
    }
  }

  private async discardUnlocked(taskId: string): Promise<void> {
    const directory = await tasksDirectory(false);
    if (!directory) return;
    await directory.removeEntry(taskStorageFilename(taskId)).catch((error: unknown) => {
      if (!(error instanceof DOMException && error.name === 'NotFoundError')) throw error;
    });
  }

  private async clearUnlocked(): Promise<void> {
    const root = await opfsRoot();
    await root.removeEntry('cakesplitter-tasks', { recursive: true }).catch((error: unknown) => {
      if (!(error instanceof DOMException && error.name === 'NotFoundError')) throw error;
    });
  }

  private enqueue<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.operationQueue.catch(() => undefined).then(operation);
    this.operationQueue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }
}

export function taskStorageFilename(taskId: string): string {
  assertIdentifier(taskId, 'Task ID');
  return `${taskId}.json`;
}

export function parsePersistedTask(text: string): PersistedTask {
  if (new TextEncoder().encode(text).byteLength > MAX_TASK_METADATA_BYTES) {
    throw new TaskStoreUnavailableError('Task metadata exceeds the local safety limit.');
  }
  let value: unknown;
  try {
    value = JSON.parse(text) as unknown;
  } catch {
    throw new TaskStoreUnavailableError('Task metadata is not valid JSON.');
  }
  return validatePersistedTask(value);
}

export function validatePersistedTask(value: unknown): PersistedTask {
  const task = assertRecord(value, 'Task metadata');
  assertExactKeys(task, [
    'schemaVersion',
    'taskId',
    'operation',
    'packageId',
    'originalFilename',
    'expectedSize',
    'expectedSha256',
    'sliceSize',
    'sliceCount',
    'completedSliceIndexes',
    'outputMode',
    'status',
    'createdAt',
    'updatedAt',
    'capability',
    'recovery',
  ]);
  if (task.schemaVersion !== TASK_SCHEMA_VERSION) {
    throw new TaskStoreUnavailableError('Task metadata schema version is unsupported.');
  }
  assertIdentifier(task.taskId, 'Task ID');
  assertOperation(task.operation);
  if (task.packageId !== undefined) assertIdentifier(task.packageId, 'Package ID');
  if (typeof task.originalFilename !== 'string') {
    throw new TaskStoreUnavailableError('Original filename must be a string.');
  }
  validatePortableFilename(task.originalFilename);
  assertSafeInteger(task.expectedSize, 'Expected size', 0);
  if (task.expectedSha256 !== undefined) validateSha256(task.expectedSha256, 'Expected task output');
  if (task.sliceSize !== undefined) assertSafeInteger(task.sliceSize, 'Slice size', 1);
  assertSafeInteger(task.sliceCount, 'Slice count', 0, MAX_SLICE_COUNT);
  const completedSliceIndexes = assertIndexArray(task.completedSliceIndexes, task.sliceCount);
  assertOutputMode(task.outputMode);
  assertTaskStatus(task.status);
  assertIsoDate(task.createdAt, 'Created timestamp');
  assertIsoDate(task.updatedAt, 'Updated timestamp');
  const capability = validateCapability(task.capability);
  const recovery = validateRecovery(task.recovery);
  if (task.operation === 'inspect' ? task.outputMode !== 'read-only' : task.outputMode === 'read-only') {
    throw new TaskStoreUnavailableError('Task operation and output mode are inconsistent.');
  }
  if (task.createdAt > task.updatedAt) {
    throw new TaskStoreUnavailableError('Task timestamps are out of order.');
  }
  return {
    schemaVersion: TASK_SCHEMA_VERSION,
    taskId: task.taskId,
    operation: task.operation,
    ...(task.packageId !== undefined ? { packageId: task.packageId } : {}),
    originalFilename: task.originalFilename,
    expectedSize: task.expectedSize,
    ...(task.expectedSha256 !== undefined ? { expectedSha256: task.expectedSha256 } : {}),
    ...(task.sliceSize !== undefined ? { sliceSize: task.sliceSize } : {}),
    sliceCount: task.sliceCount,
    completedSliceIndexes,
    outputMode: task.outputMode,
    status: task.status,
    createdAt: task.createdAt,
    updatedAt: task.updatedAt,
    capability,
    recovery,
  };
}

function validateCapability(value: unknown): CapabilitySnapshot {
  const capability = assertRecord(value, 'Capability snapshot');
  assertExactKeys(capability, ['directFolder', 'reason']);
  if (typeof capability.directFolder !== 'boolean' || typeof capability.reason !== 'string' || capability.reason.length > 1_024) {
    throw new TaskStoreUnavailableError('Capability snapshot is invalid.');
  }
  return { directFolder: capability.directFolder, reason: capability.reason };
}

function validateRecovery(value: unknown): RecoveryRequirements {
  const recovery = assertRecord(value, 'Recovery requirements');
  assertExactKeys(recovery, ['sourceFile', 'manifest', 'slices', 'outputDirectory']);
  for (const field of ['sourceFile', 'manifest', 'slices', 'outputDirectory'] as const) {
    if (typeof recovery[field] !== 'boolean') {
      throw new TaskStoreUnavailableError('Recovery requirements are invalid.');
    }
  }
  return recovery as unknown as RecoveryRequirements;
}

async function tasksDirectory(create: boolean): Promise<OpfsDirectoryHandle | undefined> {
  const root = await opfsRoot();
  try {
    return await root.getDirectoryHandle('cakesplitter-tasks', { create });
  } catch (error) {
    if (!create && error instanceof DOMException && error.name === 'NotFoundError') return undefined;
    throw error;
  }
}

async function opfsRoot(): Promise<OpfsDirectoryHandle> {
  if (typeof navigator === 'undefined' || navigator.storage === undefined) {
    throw new TaskStoreUnavailableError();
  }
  const storage = navigator.storage as StorageManager & {
    getDirectory?: () => Promise<unknown>;
  };
  if (typeof storage.getDirectory !== 'function') throw new TaskStoreUnavailableError();
  return (await storage.getDirectory()) as unknown as OpfsDirectoryHandle;
}

function assertRecord(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TaskStoreUnavailableError(`${label} must be an object.`);
  }
  return value as Record<string, unknown>;
}

function assertExactKeys(value: Record<string, unknown>, allowed: string[]): void {
  const optional = new Set(['packageId', 'expectedSha256', 'sliceSize']);
  const unexpected = Object.keys(value).filter((key) => !allowed.includes(key));
  const missing = allowed.filter((key) => !optional.has(key) && !Object.prototype.hasOwnProperty.call(value, key));
  if (unexpected.length || missing.length) {
    throw new TaskStoreUnavailableError('Task metadata fields are invalid.');
  }
}

function assertIdentifier(value: unknown, label: string): asserts value is string {
  if (typeof value !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$/u.test(value)) {
    throw new TaskStoreUnavailableError(`${label} is invalid.`);
  }
}

function assertSafeInteger(value: unknown, label: string, minimum: number, maximum = MAX_SAFE_INTEGER): asserts value is number {
  if (!Number.isSafeInteger(value) || (value as number) < minimum || (value as number) > maximum) {
    throw new TaskStoreUnavailableError(`${label} is outside the supported range.`);
  }
}

function assertIndexArray(value: unknown, sliceCount: number): number[] {
  if (!Array.isArray(value) || value.length > MAX_SLICE_COUNT) {
    throw new TaskStoreUnavailableError('Completed Slice indexes are invalid.');
  }
  const indexes = value.map((entry) => {
    if (sliceCount === 0) {
      throw new TaskStoreUnavailableError('A zero-Slice task cannot contain completed Slice indexes.');
    }
    assertSafeInteger(entry, 'Completed Slice index', 1, Math.max(1, sliceCount));
    return entry;
  });
  if (new Set(indexes).size !== indexes.length) {
    throw new TaskStoreUnavailableError('Completed Slice indexes contain duplicates.');
  }
  return indexes.sort((left, right) => left - right);
}

function assertOperation(value: unknown): asserts value is WorkerOperation {
  if (value !== 'split' && value !== 'merge' && value !== 'inspect') {
    throw new TaskStoreUnavailableError('Task operation is invalid.');
  }
}

function assertOutputMode(value: unknown): asserts value is OutputMode {
  if (value !== 'direct' && value !== 'fallback' && value !== 'read-only') {
    throw new TaskStoreUnavailableError('Task output mode is invalid.');
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
    throw new TaskStoreUnavailableError('Task status is invalid.');
  }
}

function assertIsoDate(value: unknown, label: string): asserts value is string {
  if (typeof value !== 'string' || Number.isNaN(Date.parse(value)) || new Date(value).toISOString() !== value) {
    throw new TaskStoreUnavailableError(`${label} is invalid.`);
  }
}
