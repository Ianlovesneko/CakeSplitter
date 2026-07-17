import { describe, expect, it } from 'vitest';

import {
  DIRECT_FOLDER_MODE_ENABLED,
  DirectFolderSecurityError,
  STREAM_CHUNK_SIZE,
  getDirectFolderCapabilities,
  hashBlob,
  streamBlob,
  supportsDirectFolderMode,
  validateOutputPlan,
  writeVerifiedOutput,
  type OutputSnapshot,
  type SecureOutputAdapter,
  type StagedOutput,
} from '../src/index';

describe('browser output capability', () => {
  it('fails closed when atomic no-replace publication is unavailable', () => {
    class FileHandle {
      isSameEntry() { return Promise.resolve(true); }
      move() { return Promise.resolve(); }
    }
    class DirectoryHandle {
      isSameEntry() { return Promise.resolve(true); }
    }
    class Writable {}
    const capabilities = getDirectFolderCapabilities({
      isSecureContext: true,
      showOpenFilePicker() {},
      showDirectoryPicker() {},
      FileSystemFileHandle: FileHandle,
      FileSystemDirectoryHandle: DirectoryHandle,
      FileSystemWritableFileStream: Writable,
    });
    expect(capabilities).toMatchObject({
      secureContext: true,
      openFilePicker: true,
      directoryPicker: true,
      handleIdentity: true,
      move: true,
      atomicNoReplace: false,
      supported: false,
    });
    expect(capabilities.reason).toMatch(/atomic no-replace/u);
    expect(DIRECT_FOLDER_MODE_ENABLED).toBe(false);
    expect(supportsDirectFolderMode()).toBe(false);
  });
});

describe('bounded streaming', () => {
  it('reads at most one configured chunk and awaits each consumer', async () => {
    const blob = new Blob([new Uint8Array(STREAM_CHUNK_SIZE * 2 + 7)]);
    const sizes: number[] = [];
    let active = 0;
    let maximumActive = 0;
    await streamBlob(blob, async (chunk) => {
      active += 1;
      maximumActive = Math.max(maximumActive, active);
      sizes.push(chunk.byteLength);
      await Promise.resolve();
      active -= 1;
    });
    expect(sizes).toEqual([STREAM_CHUNK_SIZE, STREAM_CHUNK_SIZE, 7]);
    expect(maximumActive).toBe(1);
  });
});

describe('secure output planning', () => {
  it('rejects duplicate and Windows-reserved output names', () => {
    expect(() => validateOutputPlan(['Cake.slice', 'cake.slice'])).toThrow(/duplicate/u);
    expect(() => validateOutputPlan(['CON'])).toThrow();
  });
});

describe('verified no-replace finalization', () => {
  it('publishes only after identity, size, and hash validation', async () => {
    const bytes = new TextEncoder().encode('verified cake bytes');
    const expectedSha256 = await hashBlob(new Blob([bytes]));
    const adapter = new MemoryAdapter();
    const result = await writeVerifiedOutput(adapter, {
      taskId: '12345678-success',
      finalName: 'cake.bin',
      expectedSize: bytes.byteLength,
      expectedSha256,
      chunks: chunks(bytes, 3),
    });
    expect(result).toEqual({
      finalName: 'cake.bin',
      size: bytes.byteLength,
      sha256: expectedSha256,
    });
    expect(adapter.published).toBe(true);
  });

  it('rejects a destination that exists before processing', async () => {
    const adapter = new MemoryAdapter();
    adapter.existingDestination = true;
    await expect(validRequest(adapter)).rejects.toMatchObject({ code: 'output_collision' });
    expect(adapter.partial).toBeUndefined();
  });

  it('preserves the partial when a destination appears during processing', async () => {
    const adapter = new MemoryAdapter();
    adapter.collisionOnSecondCheck = true;
    await expect(validRequest(adapter)).rejects.toMatchObject({ code: 'output_collision' });
    expect(adapter.published).toBe(false);
    expect(adapter.partial).toBeDefined();
  });

  it('rejects directory invalidation and partial-handle rebinding', async () => {
    const directoryChanged = new MemoryAdapter();
    directoryChanged.rebindDirectoryAfterWrite = true;
    await expect(validRequest(directoryChanged)).rejects.toMatchObject({
      code: 'directory_rebound',
    });

    const partialChanged = new MemoryAdapter();
    partialChanged.rebindPartialAfterWrite = true;
    await expect(validRequest(partialChanged)).rejects.toMatchObject({ code: 'partial_rebound' });
  });

  it('rejects failed close, permission loss, size mismatch, and checksum mismatch', async () => {
    const closeFailed = new MemoryAdapter();
    closeFailed.failClose = true;
    await expect(validRequest(closeFailed)).rejects.toMatchObject({ code: 'write_failed' });

    const permissionLost = new MemoryAdapter();
    permissionLost.failWrite = true;
    await expect(validRequest(permissionLost)).rejects.toMatchObject({ code: 'write_failed' });

    const wrongSize = new MemoryAdapter();
    wrongSize.reportedSizeDelta = 1;
    await expect(validRequest(wrongSize)).rejects.toMatchObject({ code: 'size_mismatch' });

    const wrongHash = new MemoryAdapter();
    wrongHash.reportedHash = '0'.repeat(64);
    await expect(validRequest(wrongHash)).rejects.toMatchObject({ code: 'checksum_mismatch' });
  });

  it('marks cancellation incomplete and never publishes it', async () => {
    const adapter = new MemoryAdapter();
    const bytes = new TextEncoder().encode('verified cake bytes');
    const expectedSha256 = await hashBlob(new Blob([bytes]));
    await expect(
      writeVerifiedOutput(adapter, {
        taskId: '12345678-cancel',
        finalName: 'cake.bin',
        expectedSize: bytes.byteLength,
        expectedSha256,
        chunks: chunks(bytes, 3),
        checkpoint: () => {
          throw new DOMException('cancelled', 'AbortError');
        },
      }),
    ).rejects.toMatchObject({ code: 'cancelled' });
    expect(adapter.published).toBe(false);
    expect(adapter.partial).toBeDefined();
  });

  it('rejects an adapter without a proven atomic primitive', async () => {
    const adapter = new MemoryAdapter();
    Object.defineProperty(adapter, 'atomicNoReplace', { value: false });
    await expect(validRequest(adapter)).rejects.toMatchObject({
      code: 'unsupported_finalization',
    });
  });
});

async function validRequest(adapter: SecureOutputAdapter) {
  const bytes = new TextEncoder().encode('verified cake bytes');
  return writeVerifiedOutput(adapter, {
    taskId: '12345678-security',
    finalName: 'cake.bin',
    expectedSize: bytes.byteLength,
    expectedSha256: await hashBlob(new Blob([bytes])),
    chunks: chunks(bytes, 4),
  });
}

async function* chunks(bytes: Uint8Array, size: number) {
  for (let offset = 0; offset < bytes.byteLength; offset += size) {
    yield bytes.slice(offset, Math.min(offset + size, bytes.byteLength));
  }
}

class MemoryAdapter implements SecureOutputAdapter {
  readonly atomicNoReplace = true as const;
  existingDestination = false;
  collisionOnSecondCheck = false;
  rebindDirectoryAfterWrite = false;
  rebindPartialAfterWrite = false;
  failClose = false;
  failWrite = false;
  reportedSizeDelta = 0;
  reportedHash?: string;
  published = false;
  partial?: { name: string; identity: string; bytes: number[]; closed: boolean };
  private destinationChecks = 0;
  private directoryChecks = 0;

  async directoryIdentity(): Promise<string> {
    this.directoryChecks += 1;
    return this.rebindDirectoryAfterWrite && this.directoryChecks > 1 ? 'directory-2' : 'directory-1';
  }

  async destinationExists(): Promise<boolean> {
    this.destinationChecks += 1;
    return this.existingDestination || (this.collisionOnSecondCheck && this.destinationChecks > 1);
  }

  async createPartialExclusive(name: string): Promise<StagedOutput> {
    if (this.partial) {
      throw new DirectFolderSecurityError('output_collision', 'Partial already exists');
    }
    const record = { name, identity: 'partial-identity', bytes: [] as number[], closed: false };
    this.partial = record;
    return {
      name,
      identity: record.identity,
      write: async (chunk) => {
        if (this.failWrite) {
          throw new DOMException('permission revoked', 'NotAllowedError');
        }
        record.bytes.push(...chunk);
      },
      close: async () => {
        if (this.failClose) {
          throw new Error('close failed');
        }
        record.closed = true;
      },
      abort: async () => undefined,
    };
  }

  async currentIdentity(): Promise<string | undefined> {
    if (!this.partial) return undefined;
    return this.rebindPartialAfterWrite ? 'replacement-identity' : this.partial.identity;
  }

  async snapshot(name: string): Promise<OutputSnapshot> {
    const record = this.partial;
    if (!record || (!this.published && name !== record.name)) {
      throw new Error('missing output');
    }
    const bytes = new Uint8Array(record.bytes);
    return {
      identity: record.identity,
      size: bytes.byteLength + this.reportedSizeDelta,
      sha256: this.reportedHash ?? (await hashBlob(new Blob([bytes]))),
    };
  }

  async publishNoReplace(): Promise<void> {
    if (this.existingDestination || this.collisionOnSecondCheck) {
      throw new DirectFolderSecurityError('output_collision', 'Destination exists');
    }
    this.published = true;
  }
}
