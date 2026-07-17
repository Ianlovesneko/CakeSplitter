import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import {
  MAX_TASK_METADATA_BYTES,
  parsePersistedTask,
  TaskStore,
  taskStorageFilename,
  validatePersistedTask,
  type PersistedTask,
} from './task-store';

const task: PersistedTask = {
  schemaVersion: 1,
  taskId: 'task-12345678',
  operation: 'split',
  originalFilename: 'cake.bin',
  expectedSize: 12,
  sliceSize: 4,
  sliceCount: 3,
  completedSliceIndexes: [1],
  outputMode: 'fallback',
  status: 'interrupted',
  createdAt: '2026-07-16T12:00:00.000Z',
  updatedAt: '2026-07-16T12:01:00.000Z',
  capability: { directFolder: false, reason: 'Atomic no-replace is unavailable.' },
  recovery: { sourceFile: true, manifest: false, slices: false, outputDirectory: false },
};

describe('task metadata validation', () => {
  it('round-trips bounded recovery metadata without file content', () => {
    expect(parsePersistedTask(JSON.stringify(task))).toEqual(task);
    expect(taskStorageFilename(task.taskId)).toBe('task-12345678.json');
  });

  it.each([
    ['unknown status', { ...task, status: 'success' }],
    ['unknown field', { ...task, privateFile: new Uint8Array([1]) }],
    ['duplicate completed Slice', { ...task, completedSliceIndexes: [1, 1] }],
    ['unsafe filename', { ...task, originalFilename: '../cake.bin' }],
    ['unsafe task ID', { ...task, taskId: '../task' }],
    ['impossible index', { ...task, completedSliceIndexes: [4] }],
    ['completed zero-Slice index', { ...task, sliceCount: 0, completedSliceIndexes: [1] }],
    ['read-only Split', { ...task, outputMode: 'read-only' }],
    ['write-mode Inspect', { ...task, operation: 'inspect', outputMode: 'fallback' }],
    ['reversed timestamps', { ...task, updatedAt: '2026-07-16T11:59:00.000Z' }],
  ])('rejects %s', (_label, value) => {
    expect(() => validatePersistedTask(value)).toThrow();
  });

  it('rejects malformed and oversized JSON', () => {
    expect(() => parsePersistedTask('{nope')).toThrow(/valid JSON/u);
    expect(() => parsePersistedTask(' '.repeat(MAX_TASK_METADATA_BYTES + 1))).toThrow(/safety limit/u);
  });
});

describe('Clear All persistence barrier', () => {
  let root: MemoryDirectory;
  let originalNavigator: PropertyDescriptor | undefined;

  beforeEach(() => {
    root = new MemoryDirectory();
    originalNavigator = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
    Object.defineProperty(globalThis, 'navigator', {
      configurable: true,
      value: {
        storage: {
          async estimate() {
            return { usage: 0, quota: 1024 * 1024 };
          },
          async getDirectory() {
            return root;
          },
        },
      },
    });
  });

  afterEach(() => {
    if (originalNavigator) Object.defineProperty(globalThis, 'navigator', originalNavigator);
    else Reflect.deleteProperty(globalThis, 'navigator');
  });

  it.each(['split', 'merge'] as const)(
    'rejects stale %s persistence after Clear All and keeps OPFS empty',
    async (operation) => {
      const store = new TaskStore();
      const generation = store.captureGeneration();
      const active = taskFor(operation, `task-${operation}`);
      await expect(store.save(active, generation)).resolves.toBe(true);

      await store.clear();
      await expect(
        store.save({ ...active, status: 'completed', updatedAt: '2026-07-16T12:02:00.000Z' }, generation),
      ).resolves.toBe(false);

      expect(await store.list()).toEqual([]);
      expect(await root.entryNames()).toEqual([]);
    },
  );

  it('waits for delayed persistence and then removes every task record', async () => {
    const store = new TaskStore();
    const generation = store.captureGeneration();
    const close = deferred<void>();
    const closeStarted = deferred<void>();
    root.nextClose = { release: close.promise, started: closeStarted.resolve };

    const save = store.save(task, generation);
    await closeStarted.promise;
    const clear = store.clear();
    const duringClear = store.save({ ...task, taskId: 'task-during-clear' });
    close.resolve();

    await expect(save).resolves.toBe(false);
    await clear;
    await expect(duringClear).resolves.toBe(false);
    expect(await store.list()).toEqual([]);
    expect(await root.entryNames()).toEqual([]);
  });

  it('coalesces concurrent cleanup requests into one deletion barrier', async () => {
    const store = new TaskStore();
    await store.save(task, store.captureGeneration());

    const first = store.clear();
    const second = store.clear();

    expect(second).toBe(first);
    await expect(first).resolves.toBe(store.captureGeneration());
    expect(root.removals).toBe(1);
    expect(await root.entryNames()).toEqual([]);
  });

  it('reloads empty after clearing and permits new-generation tasks', async () => {
    const store = new TaskStore();
    const oldGeneration = store.captureGeneration();
    await store.save(task, oldGeneration);
    await store.clear();

    const reloaded = new TaskStore();
    expect(await reloaded.list()).toEqual([]);
    expect(await root.entryNames()).toEqual([]);

    const next = { ...task, taskId: 'task-after-clear' };
    await expect(store.save(next, store.captureGeneration())).resolves.toBe(true);
    expect(await store.list()).toEqual([next]);
  });
});

function taskFor(operation: 'split' | 'merge', taskId: string): PersistedTask {
  if (operation === 'merge') {
    const withoutSliceSize = { ...task };
    delete withoutSliceSize.sliceSize;
    return {
      ...withoutSliceSize,
      taskId,
      operation,
      expectedSha256: 'a'.repeat(64),
    };
  }
  return {
    ...task,
    taskId,
    operation,
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

class MemoryFile {
  value = '';

  constructor(
    readonly name: string,
    private readonly owner: MemoryDirectory,
  ) {}

  async getFile(): Promise<File> {
    return new File([this.value], this.name, { type: 'application/json' });
  }

  async createWritable() {
    let pending = this.value;
    return {
      write: async (data: string) => {
        pending = data;
      },
      close: async () => {
        const barrier = this.owner.consumeCloseBarrier();
        if (barrier) {
          barrier.started();
          await barrier.release;
        }
        this.value = pending;
      },
      abort: async () => undefined,
    };
  }
}

class MemoryDirectory {
  readonly entries = new Map<string, MemoryDirectory | MemoryFile>();
  removals = 0;

  constructor(
    private readonly closeState: {
      next: { release: Promise<void>; started: () => void } | undefined;
    } = { next: undefined },
  ) {}

  get nextClose() {
    return this.closeState.next;
  }

  set nextClose(value: { release: Promise<void>; started: () => void } | undefined) {
    this.closeState.next = value;
  }

  async getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<MemoryDirectory> {
    const existing = this.entries.get(name);
    if (existing instanceof MemoryDirectory) return existing;
    if (existing || !options?.create) throw new DOMException('Directory not found.', 'NotFoundError');
    const directory = new MemoryDirectory(this.closeState);
    this.entries.set(name, directory);
    return directory;
  }

  async getFileHandle(name: string, options?: { create?: boolean }): Promise<MemoryFile> {
    const existing = this.entries.get(name);
    if (existing instanceof MemoryFile) return existing;
    if (existing || !options?.create) throw new DOMException('File not found.', 'NotFoundError');
    const file = new MemoryFile(name, this);
    this.entries.set(name, file);
    return file;
  }

  async removeEntry(name: string): Promise<void> {
    if (!this.entries.delete(name)) throw new DOMException('Entry not found.', 'NotFoundError');
    this.removals += 1;
  }

  async *values(): AsyncIterable<{ kind: 'file' | 'directory'; name: string }> {
    for (const [name, entry] of this.entries) {
      yield { kind: entry instanceof MemoryFile ? 'file' : 'directory', name };
    }
  }

  async entryNames(): Promise<string[]> {
    return [...this.entries.keys()].sort();
  }

  consumeCloseBarrier() {
    const barrier = this.closeState.next;
    this.closeState.next = undefined;
    return barrier;
  }
}
